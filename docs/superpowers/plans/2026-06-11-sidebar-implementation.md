# Sidebar 与 Tab 双模式布局 — 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 edit+ 中实现 Sidebar 与 Tabs 互斥的双模式顶部布局，包含 macOS 原生 titlebar 整合、hover 浮层、钉住、边缘拖宽与持久化；同时把 tab_bar 重构为内聚状态机，把通用菜单组件抽离。

**Architecture:** ui crate 新增 `view_mode.rs` / `popup_menu.rs` / `sidebar.rs` 三个模块，把 `tab_bar.rs` 现有散装函数重构为 `TabBarState` 内聚结构（与 `SidebarState` 对称）。app crate 新增 `sys/macos_titlebar.rs` 桥接 NSWindow `fullSizeContentView`。`ViewMode` 全局存于 `~/.edit+/settings.toml`，`pinned/width` 按工作区存于 `~/.edit+/workspace.yaml`。每帧 render_pipeline 顶层根据 view_mode 二选一调用 tab_bar / sidebar 的 `update_layout` + `vertices` + `text_positions`。

**Tech Stack:** Rust 2024 edition；wgpu + winit + cosmic-text；objc2 + objc2-app-kit + objc2-foundation（已在 workspace 依赖中）；serde + serde_yml。

---

## 文件结构

### 新增

| 文件 | 职责 |
|---|---|
| `crates/ui/src/view_mode.rs` | `pub enum ViewMode { Sidebar, Tabs }`，serde 派生。 |
| `crates/ui/src/popup_menu.rs` | 通用菜单组件：`PopupMenu` / `PopupMenuItem` / `PopupMenuAction` / vertices / text_positions / hit_test。从 tab_bar.rs 抽离。 |
| `crates/ui/src/sidebar.rs` | 侧边栏组件：`SidebarInput` / `SidebarConfig` / `SidebarState` / `SidebarAction` / `SidebarKey` / vertices / text_positions / `Visibility`。 |
| `crates/app/src/sys/mod.rs` | `pub mod macos_titlebar;` 仅在 app 内部使用。 |
| `crates/app/src/sys/macos_titlebar.rs` | NSWindow 调整：`enable_full_size_content` / `disable_full_size_content` / `traffic_light_inset`。`#[cfg(target_os = "macos")]` 之外为 stub。 |
| `crates/app/src/settings_io.rs` | `~/.edit+/settings.toml` 加载 / 保存（仅写 view_mode 字段；后续可扩展）。使用 serde_yml 暂时复用 yaml（与 workspace.yaml 一致）。 |

### 修改

| 文件 | 改动概要 |
|---|---|
| `crates/ui/src/lib.rs` | 导出 `view_mode` / `popup_menu` / `sidebar` 三个新模块。 |
| `crates/ui/src/tab_bar.rs` | 抽出 PopupMenu 后 `use crate::popup_menu::*; pub use ...;`；引入 `TabBarState` 内聚结构；旧散装函数转为 `pub(crate)`。 |
| `crates/app/src/lib.rs` | `mod sys;` 暴露内部使用。 |
| `crates/app/src/workspace.rs` | `Workspace` 持有 `tab_bar_state: TabBarState` 与 `sidebar_state: SidebarState`、`sidebar_cfg: SidebarConfig`；`PersistedWorkspace` 增加 `sidebar_pinned` / `sidebar_width` 字段。 |
| `crates/app/src/render_pipeline.rs`、`crates/app/src/app_renderer.rs` | 顶层按 `view_mode` 二选一调用渲染入口；编辑区 `editor_left_offset` 计算合并 sidebar 让位。 |
| `crates/app/src/events.rs`、`crates/app/src/input.rs`、`crates/app/src/mouse.rs` | 鼠标/键盘事件按 view_mode 转发到 `tab_bar_state` 或 `sidebar_state`；新增 Cmd+B / Esc / hover 计时驱动。 |
| `crates/app/src/app.rs` | 启动时根据 view_mode 调 `sys::macos_titlebar::enable_full_size_content`；DPI / 窗口 resize / 模式切换时同步刷新。 |

### 不修改的文件

- `crates/core/**`（与 UI 无关）
- `crates/render/**`、`crates/shaping/**`（不需要新原语）
- `crates/ui/src/{viewport,layout,decorations,gutter,scrollbar,status_bar,search_bar,settings,theme,render_geom}.rs`（除非阶段 9 修 bug 时局部碰到）

---

## 阶段总览

| # | 阶段 | 关键交付 |
|---|---|---|
| 1 | 抽离 popup_menu | 通用菜单组件，零行为变化 |
| 2 | tab_bar 内聚化 | `TabBarState`，零行为变化 |
| 3 | ViewMode 枚举 + 持久化 | settings_io / workspace 字段 |
| 4 | sidebar 骨架 | 静态侧边栏，强制 Pinned，可点切 tab |
| 5 | macOS titlebar 桥接 | NSWindow fullSizeContentView |
| 6 | hover 状态机 | 4px 热区 + 150/300ms 延时 + Esc |
| 7 | 边缘拖拽改宽 + 持久化 | EdgeDragState + on_drag_end 写盘 |
| 8 | 设置菜单 | popup 菜单切模式 / 打开 settings.toml |
| 9 | 边界打磨 + 手动验证 | 极窄窗口、空 tabs、emoji 文件名 |

每阶段独立可编译可测试。

---

# 阶段 1：抽离 popup_menu

**Goal**: 把 tab_bar.rs 中 `PopupMenu` 系列代码搬到 `crates/ui/src/popup_menu.rs`；行为零变化，所有 tab_bar 测试保持绿。

### Task 1.1：创建空模块并接通 lib.rs

**Files:**
- Create: `crates/ui/src/popup_menu.rs`
- Modify: `crates/ui/src/lib.rs`

- [ ] **Step 1: 创建 popup_menu.rs 空模块**

写入 `crates/ui/src/popup_menu.rs`：

```rust
//! Generic popup menu component shared by tab_bar and sidebar.
//!
//! Provides items, hit-testing, and vertex generation. Hosts (tab_bar /
//! sidebar) hold an `Option<PopupMenu>` and forward clicks / draws.
```

- [ ] **Step 2: 在 ui::lib.rs 注册新模块**

修改 `crates/ui/src/lib.rs`，在 `pub mod tab_bar;` 之前插入 `pub mod popup_menu;`。

- [ ] **Step 3: 编译验证**

Run: `cargo build -p edit-plus-ui`
Expected: 0 错误。

- [ ] **Step 4: 提交**

```bash
git add crates/ui/src/popup_menu.rs crates/ui/src/lib.rs
git commit -m "ui: add empty popup_menu module"
```

### Task 1.2：迁移类型定义

**Files:**
- Modify: `crates/ui/src/popup_menu.rs`
- Modify: `crates/ui/src/tab_bar.rs:962-1083`

- [ ] **Step 1: 把 PopupMenu / PopupMenuItem / PopupMenuAction / ContextMenuAction 搬到 popup_menu.rs**

用 Read 读取 `crates/ui/src/tab_bar.rs:960-1085`（含 `pub enum ContextMenuAction`、`pub enum PopupMenuAction`、`pub struct PopupMenuItem`、`pub struct PopupMenu`、其 `impl`）。

完整复制到 `crates/ui/src/popup_menu.rs`，并在文件顶部加：

```rust
use crate::settings::Settings;
```

`PopupMenu::overflow` 与 `PopupMenu::context` 内部用到 `crate::tab_bar::TabBarLayout` 与 `TabBarCtx`：先把 `overflow` 与 `context` 留在 `tab_bar.rs`（它们是 tab_bar 专属的工厂），只把 `PopupMenu` / `PopupMenuItem` / `PopupMenuAction` / `ContextMenuAction` 类型搬过来；`hit_test` 跟着 `PopupMenu` 一起搬。

- [ ] **Step 2: 在 tab_bar.rs 改为重新导入 + 兼容 re-export**

把 `crates/ui/src/tab_bar.rs:960-1085` 中已搬走的类型定义删除；在文件顶部 `use crate::popup_menu::{PopupMenu, PopupMenuItem, PopupMenuAction, ContextMenuAction};`，并在文件靠近顶部加：

```rust
pub use crate::popup_menu::{
    ContextMenuAction, PopupMenu, PopupMenuAction, PopupMenuItem,
};
```

让外部（`actions.rs` / `events.rs` / `app_renderer.rs`）继续通过 `ui::tab_bar::PopupMenu` 这个旧路径访问。

- [ ] **Step 3: 编译并跑 ui 全部测试**

Run: `cargo test -p edit-plus-ui`
Expected: 所有测试 PASS（含 tab_bar 现有 case：`pm = PopupMenu::context(...)` 之类）。

- [ ] **Step 4: 编译 app**

Run: `cargo build -p edit-plus-app`
Expected: 0 错误。

- [ ] **Step 5: 提交**

```bash
git add crates/ui/src/popup_menu.rs crates/ui/src/tab_bar.rs
git commit -m "ui: extract PopupMenu types into popup_menu module"
```

### Task 1.3：迁移渲染与命中函数

**Files:**
- Modify: `crates/ui/src/popup_menu.rs`
- Modify: `crates/ui/src/tab_bar.rs:1184-1370`

- [ ] **Step 1: 把 popup_menu_vertices 与 popup_menu_text_positions 搬过来**

用 Read 读取 `crates/ui/src/tab_bar.rs:1184` 附近的 `pub fn popup_menu_vertices(...)`，搬到 `popup_menu.rs`。其中用到的 `TabBarCtx`：在 popup_menu 模块里改用通用参数 `screen_w: f32, screen_h: f32`，函数签名改为：

```rust
pub fn popup_menu_vertices(
    menu: &PopupMenu,
    theme: &Theme,
    screen_w: f32,
    screen_h: f32,
    mouse_ndc: [f32; 2],
) -> Vec<render::GlyphVertex> { /* 原实现，去掉 ctx */ }

pub fn popup_menu_text_positions(
    menu: &PopupMenu,
    screen_w: f32,
    screen_h: f32,
) -> Vec<TextPosition> { /* 原实现 */ }
```

`Theme` 位于 `crate::theme::Theme`；`TextPosition` 位于 tab_bar 内部，需要把 `TextPosition` 也搬到 popup_menu 或提到 lib.rs 公共位置。**先搬 TextPosition 到 popup_menu**（如果只用于 popup），并 re-export 自 tab_bar 以保持 app 层不变。

- [ ] **Step 2: 在 tab_bar.rs 改为薄壳转发**

```rust
pub use crate::popup_menu::{popup_menu_text_positions, popup_menu_vertices};
```

删除 tab_bar.rs 中原 `popup_menu_vertices` / `popup_menu_text_positions` 实现。

- [ ] **Step 3: 调整 app_renderer.rs 调用**

`crates/app/src/app_renderer.rs:14-18` 与 `:655 / :660` 调用处把第三参数 `&ctx` 拆为 `screen_w, screen_h`：

```rust
// before
ui::tab_bar::popup_menu_text_positions(menu, &ctx);
// after  (旧 re-export 仍可用)
ui::tab_bar::popup_menu_text_positions(menu, screen_w, screen_h);
```

`ctx` 旧值为 `TabBarCtx { screen_w, screen_h }`，直接拆字段即可。

- [ ] **Step 4: 跑全套测试**

Run: `cargo test -p edit-plus-ui -p edit-plus-app`
Expected: 所有测试 PASS。

- [ ] **Step 5: 编译 release 验证**

Run: `cargo build --release -p edit-plus-app`
Expected: 0 错误。

- [ ] **Step 6: 提交**

```bash
git add crates/ui/src/popup_menu.rs crates/ui/src/tab_bar.rs crates/app/src/app_renderer.rs
git commit -m "ui: move popup menu rendering into popup_menu module"
```

### Task 1.4：补充 popup_menu 单测

**Files:**
- Modify: `crates/ui/src/popup_menu.rs`

- [ ] **Step 1: 写第一个测试 — hit_test 命中分隔符返回 None**

在 `popup_menu.rs` 末尾加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_menu_with_separator() -> PopupMenu {
        let items = vec![
            PopupMenuItem { label: "A".into(), is_active: false, is_separator: false, action: PopupMenuAction::Custom(1) },
            PopupMenuItem { label: "".into(), is_active: false, is_separator: true,  action: PopupMenuAction::Custom(0) },
            PopupMenuItem { label: "B".into(), is_active: false, is_separator: false, action: PopupMenuAction::Custom(2) },
        ];
        let item_rects = vec![
            [-0.4, 0.4, 0.4, 0.3],
            [-0.4, 0.4, 0.3, 0.28],
            [-0.4, 0.4, 0.28, 0.18],
        ];
        PopupMenu { items, item_rects, menu_rect: [-0.4, 0.4, 0.4, 0.18] }
    }

    #[test]
    fn popup_menu_hit_test_separator_skipped() {
        let pm = make_menu_with_separator();
        // y=0.29 落在 separator rect 内
        assert!(pm.hit_test(0.0, 0.29).is_none());
    }

    #[test]
    fn popup_menu_hit_test_basic() {
        let pm = make_menu_with_separator();
        let action = pm.hit_test(0.0, 0.35).copied();
        assert!(matches!(action, Some(PopupMenuAction::Custom(1))));
    }
}
```

> 先读 popup_menu.rs 中现有 `PopupMenuAction` 看是否已有 `Custom(u32)` 变体；如果没有，先在 enum 里加：
> ```rust
> Custom(u32),
> ```

- [ ] **Step 2: 跑测试确认初状失败原因**

Run: `cargo test -p edit-plus-ui popup_menu_hit_test_separator_skipped`
Expected: 若 `Custom` 变体未加 → 编译失败；先加变体；再跑应该 PASS（PopupMenu 现有 hit_test 已经跳分隔符）。

- [ ] **Step 3: 跑全部 ui 测试**

Run: `cargo test -p edit-plus-ui`
Expected: 全 PASS。

- [ ] **Step 4: 提交**

```bash
git add crates/ui/src/popup_menu.rs
git commit -m "ui: add popup_menu hit-test unit tests + Custom action variant"
```

---

# 阶段 2：tab_bar 内聚化为 TabBarState

**Goal**: 把 tab_bar 散装函数（`layout_tabs` / `hit_test` / `tab_bar_vertices` / `set_preview_tab` / `max_tab_scroll` / `clamp_tab_scroll`）+ app 层散落状态（`workspace.tab_layout` / `workspace.hovered_tab_index` / `workspace.preview_tab_index` / `scroll_offset`）收口为 `TabBarState`。行为零变化。

### Task 2.1：定义 TabBarInput / TabBarAction / TabBarState 骨架

**Files:**
- Modify: `crates/ui/src/tab_bar.rs`

- [ ] **Step 1: 写新结构定义（先编译通过，方法体走旧函数）**

在 `tab_bar.rs` 现有内容之后加：

```rust
use std::collections::HashSet;

pub struct TabBarInput<'a> {
    pub tabs: &'a [TabInfo],
    pub active_index: Option<usize>,
    pub pinned_indices: &'a HashSet<usize>,
    pub back_enabled: bool,
    pub forward_enabled: bool,
    pub screen_w: f32,
    pub screen_h: f32,
}

#[derive(Debug, Clone)]
pub enum TabBarAction {
    SwitchTab(usize),
    CloseTab(usize),
    NewEmptyTab,
    NavigateBack,
    NavigateForward,
    OpenContextMenu { tab_index: usize, anchor: [f32; 2] },
    OpenOverflowMenu,
    Context { action: ContextMenuAction, tab_index: usize },
    ScrollLeft,
    ScrollRight,
}

#[derive(Default)]
pub struct TabBarState {
    layout: Option<TabBarLayout>,
    scroll_offset: f32,
    hovered_index: Option<usize>,
    preview_index: Option<usize>,
    open_menu: Option<crate::popup_menu::PopupMenu>,
}

impl TabBarState {
    pub fn new() -> Self { Self::default() }

    pub fn current_layout(&self) -> Option<&TabBarLayout> { self.layout.as_ref() }
    pub fn open_menu(&self) -> Option<&crate::popup_menu::PopupMenu> { self.open_menu.as_ref() }
    pub fn set_open_menu(&mut self, menu: Option<crate::popup_menu::PopupMenu>) { self.open_menu = menu; }
    pub fn scroll_offset(&self) -> f32 { self.scroll_offset }
    pub fn set_scroll_offset(&mut self, off: f32) { self.scroll_offset = off; }
    pub fn hovered_index(&self) -> Option<usize> { self.hovered_index }
    pub fn set_hovered_index(&mut self, idx: Option<usize>) { self.hovered_index = idx; }
    pub fn preview_index(&self) -> Option<usize> { self.preview_index }
    pub fn set_preview_index(&mut self, idx: Option<usize>) { self.preview_index = idx; }
}
```

- [ ] **Step 2: 编译**

Run: `cargo build -p edit-plus-ui`
Expected: 0 错误。

- [ ] **Step 3: 提交**

```bash
git add crates/ui/src/tab_bar.rs
git commit -m "ui: introduce TabBarState/Input/Action skeleton"
```

### Task 2.2：实现 update_layout / vertices / text_positions / hit_test 方法

**Files:**
- Modify: `crates/ui/src/tab_bar.rs`

- [ ] **Step 1: 给 TabBarState 加 update_layout**

在 `impl TabBarState` 中加：

```rust
pub fn update_layout(
    &mut self,
    input: &TabBarInput<'_>,
    mut shaper: Option<&mut shaping::Shaper>,
) {
    let ctx = TabBarCtx { screen_w: input.screen_w, screen_h: input.screen_h };
    let mut layout = layout_tabs(
        input.tabs,
        input.active_index.unwrap_or(0),
        &ctx,
        tab_bar_height(),
        input.pinned_indices,
        input.back_enabled,
        input.forward_enabled,
        self.scroll_offset,
        shaper.as_deref_mut(),
    );
    set_preview_tab(&mut layout, self.preview_index);
    self.layout = Some(layout);
}
```

- [ ] **Step 2: 加 vertices / text_positions / hit_test 转发**

```rust
pub fn vertices(
    &self,
    active_index: Option<usize>,
    theme: &Theme,
    screen_w: f32,
    screen_h: f32,
) -> Vec<render::GlyphVertex> {
    let Some(layout) = &self.layout else { return Vec::new(); };
    let ctx = TabBarCtx { screen_w, screen_h };
    tab_bar_vertices(layout, active_index.unwrap_or(0), theme, &ctx, self.hovered_index)
}

pub fn text_positions(&self, font_size: f32, screen_w: f32, screen_h: f32) -> Vec<TextPosition> {
    let Some(layout) = &self.layout else { return Vec::new(); };
    let ctx = TabBarCtx { screen_w, screen_h };
    tab_bar_text_positions(layout, &ctx, tab_bar_height(), font_size)
}

pub fn hit_test_at(&self, px: f32, py: f32, screen_w: f32, screen_h: f32) -> Option<TabHit> {
    let layout = self.layout.as_ref()?;
    let ctx = TabBarCtx { screen_w, screen_h };
    hit_test(px, py, layout, &ctx)
}

pub fn max_scroll(&self, doc_count: usize, screen_w: f32, screen_h: f32) -> f32 {
    let ctx = TabBarCtx { screen_w, screen_h };
    max_tab_scroll(doc_count, &ctx, tab_bar_height())
}

pub fn clamp_scroll(&mut self, off: f32, max: f32) {
    self.scroll_offset = clamp_tab_scroll(off, max);
}
```

- [ ] **Step 3: 编译**

Run: `cargo build -p edit-plus-ui`
Expected: 0 错误。

- [ ] **Step 4: 写新增回归测试**

在 `tab_bar.rs` `mod tests` 末尾加：

```rust
#[test]
fn tab_bar_state_scroll_clamp() {
    let mut s = TabBarState::new();
    s.set_scroll_offset(99999.0);
    s.clamp_scroll(s.scroll_offset(), 200.0);
    assert!((s.scroll_offset() - 200.0).abs() < 0.5);
}

#[test]
fn tab_bar_state_hover_transition() {
    let mut s = TabBarState::new();
    assert_eq!(s.hovered_index(), None);
    s.set_hovered_index(Some(3));
    assert_eq!(s.hovered_index(), Some(3));
    s.set_hovered_index(None);
    assert_eq!(s.hovered_index(), None);
}
```

- [ ] **Step 5: 跑全部 ui 测试**

Run: `cargo test -p edit-plus-ui`
Expected: 全 PASS。

- [ ] **Step 6: 提交**

```bash
git add crates/ui/src/tab_bar.rs
git commit -m "ui: implement TabBarState rendering and hit-test methods"
```

### Task 2.3：实现 on_click / on_scroll / on_mouse_move 事件入口

**Files:**
- Modify: `crates/ui/src/tab_bar.rs`

- [ ] **Step 1: 加事件方法**

在 `impl TabBarState` 中加：

```rust
pub fn on_mouse_move(&mut self, px: f32, py: f32, screen_w: f32, screen_h: f32) {
    self.hovered_index = match self.hit_test_at(px, py, screen_w, screen_h) {
        Some(TabHit::Tab(idx)) => Some(idx),
        _ => None,
    };
}

pub fn on_mouse_leave(&mut self) {
    self.hovered_index = None;
}

pub fn on_click(
    &mut self,
    px: f32,
    py: f32,
    button: MouseButton,
    screen_w: f32,
    screen_h: f32,
) -> Option<TabBarAction> {
    let hit = self.hit_test_at(px, py, screen_w, screen_h)?;
    match (hit, button) {
        (TabHit::Tab(idx), MouseButton::Left) => Some(TabBarAction::SwitchTab(idx)),
        (TabHit::Tab(idx), MouseButton::Right) => {
            let ndc_x = px / screen_w * 2.0 - 1.0;
            let ndc_y = 1.0 - py / screen_h * 2.0;
            Some(TabBarAction::OpenContextMenu { tab_index: idx, anchor: [ndc_x, ndc_y] })
        }
        (TabHit::Close(idx), MouseButton::Left) => Some(TabBarAction::CloseTab(idx)),
        (TabHit::NewTab, _) => Some(TabBarAction::NewEmptyTab),
        (TabHit::ScrollLeft, _) => Some(TabBarAction::ScrollLeft),
        (TabHit::ScrollRight, _) => Some(TabBarAction::ScrollRight),
        (TabHit::Dropdown, _) => Some(TabBarAction::OpenOverflowMenu),
        _ => None,
    }
}

pub fn on_scroll(&mut self, dx: f32, doc_count: usize, screen_w: f32, screen_h: f32) {
    let max = self.max_scroll(doc_count, screen_w, screen_h);
    self.scroll_offset = clamp_tab_scroll(self.scroll_offset + dx, max);
}
```

`MouseButton` 暂时声明为：

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MouseButton { Left, Right, Middle }
```

- [ ] **Step 2: 编译**

Run: `cargo build -p edit-plus-ui`
Expected: 0 错误。

- [ ] **Step 3: 提交**

```bash
git add crates/ui/src/tab_bar.rs
git commit -m "ui: add TabBarState event entrypoints (mouse + click + scroll)"
```

### Task 2.4：迁移 app 层调用面到 TabBarState

**Files:**
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/app.rs`

- [ ] **Step 1: 在 Workspace 增加 tab_bar_state 字段**

修改 `crates/app/src/workspace.rs`：

```rust
use ui::tab_bar::TabBarState;

pub(crate) struct Workspace {
    // ... 现有字段
    pub(crate) tab_bar_state: TabBarState,
    // 旧字段 tab_layout / hovered_tab_index / preview_tab_index 保留，逐步迁移
}
```

`Workspace::new()` 里 `tab_bar_state: TabBarState::new(),`。

- [ ] **Step 2: 把 app_renderer.rs 中读 layout 的位置改为读 tab_bar_state**

`crates/app/src/app_renderer.rs:472` 附近：

```rust
let input = ui::tab_bar::TabBarInput {
    tabs: &tab_infos,
    active_index: Some(self.workspace.active_index),
    pinned_indices: &self.workspace.pinned_indices,
    back_enabled: self.workspace.has_back_history(),
    forward_enabled: self.workspace.has_forward_history(),
    screen_w,
    screen_h,
};
self.workspace.tab_bar_state.set_preview_index(self.workspace.preview_tab_index);
self.workspace.tab_bar_state.set_scroll_offset(self.workspace.tab_layout
    .as_ref().map(|l| l.scroll_offset).unwrap_or(0.0));
self.workspace.tab_bar_state.update_layout(&input, shaper);
self.workspace.tab_layout = self.workspace.tab_bar_state.current_layout().cloned();
```

`pinned_indices`：如果 Workspace 没有该字段，先在 Workspace 暴露 getter；本步骤旨在把 layout 写入 TabBarState，旧字段 `tab_layout` 暂保留以兼容下游。

- [ ] **Step 3: 把 vertices 调用迁过去**

`crates/app/src/app_renderer.rs:576`：

```rust
// before
tab_bar::tab_bar_vertices(self.workspace.tab_layout.as_ref().unwrap(), self.workspace.active_index, &self.current_theme, &ctx, self.workspace.hovered_tab_index)
// after
self.workspace.tab_bar_state.vertices(Some(self.workspace.active_index), &self.current_theme, screen_w, screen_h)
```

类似地把 `tab_bar_text_positions` 调用改为 `self.workspace.tab_bar_state.text_positions(font_size, screen_w, screen_h)`。

- [ ] **Step 4: events.rs 把 hit_test 改为 state 方法**

`crates/app/src/events.rs:73-90`、`:248-252` 的 `ui::tab_bar::hit_test(px, py, layout, &ctx)` 调用改为：

```rust
let hit = workspace.tab_bar_state.hit_test_at(px, py, screen_w, screen_h);
```

或用 `workspace.tab_bar_state.on_click(px, py, MouseButton::Left, screen_w, screen_h)` 直接拿到 action，再 map 为 AppAction。**保持原 AppAction 路径不变**，只是 hit-test 不再走 layout 字段。

- [ ] **Step 5: 跑测试**

Run: `cargo test -p edit-plus-app -p edit-plus-ui`
Expected: 全 PASS。

- [ ] **Step 6: 启动手测**

Run: `cargo run -p edit-plus-app -- assets/samples/medium_ascii_5mb.txt`
预期：tab 栏行为完全和重构前一致（点切 / 关闭按钮 / 滚动 / 右键菜单 / +按钮 / overflow dropdown）。

- [ ] **Step 7: 提交**

```bash
git add crates/app/src/workspace.rs crates/app/src/app_renderer.rs crates/app/src/events.rs crates/app/src/app.rs
git commit -m "app: route tab bar layout/hit/render through TabBarState"
```

### Task 2.5：清理 workspace 旧散落字段

**Files:**
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/app.rs`

- [ ] **Step 1: 删除 Workspace.tab_layout / hovered_tab_index / preview_tab_index**

把它们都改为通过 `tab_bar_state` 访问：

```rust
// Workspace 上的方法改为 thin wrapper
pub(crate) fn tab_layout(&self) -> Option<&ui::tab_bar::TabBarLayout> {
    self.tab_bar_state.current_layout()
}
pub(crate) fn set_hovered_tab(&mut self, idx: Option<usize>) {
    self.tab_bar_state.set_hovered_index(idx);
}
```

更新所有 `self.workspace.tab_layout` / `self.workspace.hovered_tab_index` / `self.workspace.preview_tab_index` 引用。

- [ ] **Step 2: 编译并跑测试**

Run: `cargo test -p edit-plus-app -p edit-plus-ui`
Expected: 全 PASS。

- [ ] **Step 3: 检查 clippy**

Run: `cargo clippy -p edit-plus-ui -p edit-plus-app -- -D warnings`
Expected: 无 warning。

- [ ] **Step 4: 提交**

```bash
git add crates/app/src/workspace.rs crates/app/src/events.rs crates/app/src/app.rs
git commit -m "app: collapse tab bar scratch fields into TabBarState"
```

---

# 阶段 3：ViewMode 枚举 + Settings/workspace 字段持久化

**Goal**: 引入 `ViewMode` 与 `SidebarConfig`，并落地到 `~/.edit+/settings.toml` 与 `~/.edit+/workspace.yaml`，实现读写往返。**本阶段交付时默认值临时设为 `ViewMode::Tabs`**（sidebar 还没实现）。

### Task 3.1：在 ui 暴露 ViewMode 与 SidebarConfig 类型

**Files:**
- Create: `crates/ui/src/view_mode.rs`
- Modify: `crates/ui/src/lib.rs`
- Modify: `crates/ui/src/sidebar.rs`（占位，先空模块）
- Modify: `crates/ui/src/settings.rs`

- [ ] **Step 1: 创建 view_mode.rs**

写入 `crates/ui/src/view_mode.rs`：

```rust
//! Top-level view mode (Sidebar vs Tabs). Persisted in settings.toml.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ViewMode {
    Sidebar,
    Tabs,
}

impl Default for ViewMode {
    fn default() -> Self { ViewMode::Tabs } // 阶段 4 完成后改为 Sidebar
}
```

- [ ] **Step 2: 创建 sidebar.rs 占位**

写入 `crates/ui/src/sidebar.rs`：

```rust
//! Sidebar component (Stage 4+).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidebarConfig {
    pub pinned: bool,
    pub width: f32,
}

impl SidebarConfig {
    pub fn new_default(dpi_scale: f32) -> Self {
        Self {
            pinned: false,
            width: 220.0 * dpi_scale,
        }
    }

    pub fn clamp_width(&mut self, dpi_scale: f32) {
        let lo = 160.0 * dpi_scale;
        let hi = 400.0 * dpi_scale;
        self.width = self.width.clamp(lo, hi);
    }
}
```

- [ ] **Step 3: 在 ui::lib.rs 注册**

修改 `crates/ui/src/lib.rs`，加：

```rust
pub mod view_mode;
pub mod sidebar;
```

- [ ] **Step 4: 在 Settings 增加 view_mode 字段**

修改 `crates/ui/src/settings.rs`：

```rust
use crate::view_mode::ViewMode;

pub struct Settings {
    // ... 既有字段
    pub view_mode: ViewMode,
}
```

`Settings::new()` 中加 `view_mode: ViewMode::default(),`，并在 `default_settings` 测试中加 `assert_eq!(s.view_mode, ViewMode::Tabs);`。

- [ ] **Step 5: 加 ui 单测确认 SidebarConfig clamp**

在 `crates/ui/src/sidebar.rs` 末尾加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_width_below_min() {
        let mut c = SidebarConfig { pinned: false, width: 50.0 };
        c.clamp_width(1.0);
        assert_eq!(c.width, 160.0);
    }

    #[test]
    fn clamp_width_above_max() {
        let mut c = SidebarConfig { pinned: false, width: 9999.0 };
        c.clamp_width(2.0);
        assert_eq!(c.width, 800.0);
    }

    #[test]
    fn clamp_width_within_range_unchanged() {
        let mut c = SidebarConfig { pinned: false, width: 300.0 };
        c.clamp_width(1.0);
        assert_eq!(c.width, 300.0);
    }
}
```

- [ ] **Step 6: 跑测试**

Run: `cargo test -p edit-plus-ui`
Expected: 全 PASS。

- [ ] **Step 7: 提交**

```bash
git add crates/ui/src/view_mode.rs crates/ui/src/sidebar.rs crates/ui/src/lib.rs crates/ui/src/settings.rs
git commit -m "ui: introduce ViewMode + SidebarConfig stub types"
```

### Task 3.2：app 层 settings_io 读写 view_mode

**Files:**
- Create: `crates/app/src/settings_io.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: 创建 settings_io.rs**

```rust
//! Persistence for ~/.edit+/settings.yaml (kept yaml to match workspace.yaml format).

use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use ui::view_mode::ViewMode;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct PersistedSettings {
    pub view_mode: ViewMode,
}

fn settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".edit+").join("settings.yaml")
}

pub(crate) fn load() -> PersistedSettings {
    let path = settings_path();
    let yaml = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return PersistedSettings::default(),
    };
    serde_yml::from_str(&yaml).unwrap_or_default()
}

pub(crate) fn save(settings: &PersistedSettings) {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(yaml) = serde_yml::to_string(settings) {
        let _ = std::fs::write(&path, yaml);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_view_mode_is_tabs() {
        assert_eq!(PersistedSettings::default().view_mode, ViewMode::Tabs);
    }

    #[test]
    fn persisted_settings_roundtrip() {
        let s = PersistedSettings { view_mode: ViewMode::Sidebar };
        let yaml = serde_yml::to_string(&s).unwrap();
        let parsed: PersistedSettings = serde_yml::from_str(&yaml).unwrap();
        assert_eq!(parsed.view_mode, ViewMode::Sidebar);
    }

    #[test]
    fn missing_field_falls_back_to_default() {
        let yaml = "{}";
        let parsed: PersistedSettings = serde_yml::from_str(yaml).unwrap();
        assert_eq!(parsed.view_mode, ViewMode::Tabs);
    }
}
```

- [ ] **Step 2: 在 app/src/lib.rs 注册**

把 `pub(crate) mod settings_io;` 加进 `crates/app/src/lib.rs`。

- [ ] **Step 3: 跑测试**

Run: `cargo test -p edit-plus-app settings_io::tests`
Expected: 3/3 PASS。

- [ ] **Step 4: 提交**

```bash
git add crates/app/src/settings_io.rs crates/app/src/lib.rs
git commit -m "app: add settings_io for view_mode persistence"
```

### Task 3.3：app 启动时加载 view_mode

**Files:**
- Modify: `crates/app/src/app.rs`

- [ ] **Step 1: 在 App 初始化路径加载 settings**

定位 `crates/app/src/app.rs` 中 `Settings::init(...)` 的调用（搜索 `Settings::init`）。在它之后插入：

```rust
let persisted = crate::settings_io::load();
{
    let mut s = Settings::get_mut();
    s.view_mode = persisted.view_mode;
}
```

- [ ] **Step 2: 加 smoke 测试覆盖加载**

如果 `crates/app/tests/smoke.rs` 存在，新增：

```rust
#[test]
fn settings_load_does_not_panic() {
    let _ = edit_plus_app::settings_io::load();
}
```

如果 `settings_io` 是 `pub(crate)`，把 load 暂时改为 `pub` 或在 `crates/app/src/lib.rs` re-export `pub use settings_io::load as load_settings;`，仅用于测试可见性。**最简方案：在 app crate 内 `tests/` 目录下的集成测试通过 `pub` 入口访问；模块层级只在阶段后续可见。**

跳过此 step 如阶段 5 之前不便测试，留到 §6 集成测试统一覆盖。

- [ ] **Step 3: 编译启动**

Run: `cargo run -p edit-plus-app -- assets/samples/medium_ascii_5mb.txt`
预期：无 view_mode 持久化文件时，进入 Tabs 模式（行为不变）。

- [ ] **Step 4: 提交**

```bash
git add crates/app/src/app.rs
git commit -m "app: load view_mode from settings.yaml at startup"
```

### Task 3.4：workspace.yaml 增加 sidebar_pinned / sidebar_width 字段

**Files:**
- Modify: `crates/app/src/workspace.rs`

- [ ] **Step 1: 在 PersistedWorkspace 增加字段**

定位 `crates/app/src/workspace.rs` 中 `PersistedWorkspace` 定义（搜索 `struct PersistedWorkspace`）。加：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct PersistedWorkspace {
    pub version: u32,
    pub active_index: usize,
    pub tabs: Vec<PersistedTab>,
    pub sidebar_pinned: bool,
    pub sidebar_width: Option<f32>,  // None → 默认 220 * dpi_scale
}
```

- [ ] **Step 2: 在 Workspace 增加 sidebar_cfg 字段**

```rust
use ui::sidebar::SidebarConfig;

pub(crate) struct Workspace {
    // 已有字段
    pub(crate) sidebar_cfg: SidebarConfig,
}

impl Workspace {
    pub(crate) fn new() -> Self {
        let dpi = Settings::get_static().dpi_scale;
        Self {
            // ...
            sidebar_cfg: SidebarConfig::new_default(dpi),
        }
    }
}
```

- [ ] **Step 3: save_snapshot / load_snapshot 处理新字段**

`workspace.rs:391-435 save_snapshot` 末尾，把 `PersistedWorkspace { ... }` 修改为：

```rust
let snap = PersistedWorkspace {
    version: Self::WORKSPACE_VERSION,
    active_index: self.active_index,
    tabs,
    sidebar_pinned: self.sidebar_cfg.pinned,
    sidebar_width: Some(self.sidebar_cfg.width),
};
```

`workspace.rs:439 load_snapshot` 中构造 Workspace 时：

```rust
let dpi = Settings::get_static().dpi_scale;
let mut sidebar_cfg = SidebarConfig {
    pinned: snap.sidebar_pinned,
    width: snap.sidebar_width.unwrap_or(220.0 * dpi),
};
sidebar_cfg.clamp_width(dpi);
// ... 在最后构造的 Workspace 里赋值 sidebar_cfg
```

- [ ] **Step 4: 写持久化往返集成测试**

在 `crates/app/src/workspace.rs` `mod tests`（如不存在则添加）加：

```rust
#[cfg(test)]
mod sidebar_persistence_tests {
    use super::*;

    #[test]
    fn persisted_workspace_roundtrip_with_sidebar_fields() {
        let snap = PersistedWorkspace {
            version: Workspace::WORKSPACE_VERSION,
            active_index: 0,
            tabs: vec![],
            sidebar_pinned: true,
            sidebar_width: Some(280.0),
        };
        let yaml = serde_yml::to_string(&snap).unwrap();
        let parsed: PersistedWorkspace = serde_yml::from_str(&yaml).unwrap();
        assert!(parsed.sidebar_pinned);
        assert_eq!(parsed.sidebar_width, Some(280.0));
    }

    #[test]
    fn persisted_workspace_missing_sidebar_fields_default() {
        let yaml = r#"
version: 1
active_index: 0
tabs: []
"#;
        let parsed: PersistedWorkspace = serde_yml::from_str(yaml).unwrap();
        assert!(!parsed.sidebar_pinned);
        assert_eq!(parsed.sidebar_width, None);
    }
}
```

- [ ] **Step 5: 跑测试**

Run: `cargo test -p edit-plus-app workspace`
Expected: 全 PASS。

- [ ] **Step 6: 提交**

```bash
git add crates/app/src/workspace.rs
git commit -m "app: persist sidebar pinned/width in workspace.yaml"
```

---

# 阶段 4：sidebar 模块骨架

**Goal**: 实现最小 sidebar：在 `view_mode = Sidebar` 且强制 `Visibility::Pinned` 下，渲染顶部 header（含 ☰）、新建按钮、文件项列表、设置按钮；左键点击文件项 → 切 tab；编辑区水平让位 `width` 像素。**本阶段不实现 hover、不实现拖拽改宽**。完成后将 `ViewMode::default()` 改为 `Sidebar`。

### Task 4.1：定义 SidebarInput / SidebarAction / SidebarKey / Visibility

**Files:**
- Modify: `crates/ui/src/sidebar.rs`

- [ ] **Step 1: 加类型定义**

```rust
use std::path::PathBuf;
use crate::tab_bar::TabInfo;
use crate::popup_menu::{ContextMenuAction, PopupMenu};
use crate::theme::Theme;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Visibility { Hidden, HoverPeek, Pinned }

pub struct SidebarInput<'a> {
    pub tabs: &'a [TabInfo],
    pub active_index: Option<usize>,
    pub screen_w: f32,
    pub screen_h: f32,
    pub traffic_light_inset: (f32, f32), // (left, top) — 阶段 5 才会非零
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SidebarKey { TogglePin, Escape }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MouseButton { Left, Right }

#[derive(Debug, Clone)]
pub enum SidebarAction {
    SwitchTab(usize),
    NewDocument,
    OpenSettingsMenu,
    ToggleViewMode,
    TogglePin,
    SetWidth(f32),
    Context { action: ContextMenuAction, tab_index: usize },
}
```

- [ ] **Step 2: 编译**

Run: `cargo build -p edit-plus-ui`
Expected: 0 错误。

- [ ] **Step 3: 提交**

```bash
git add crates/ui/src/sidebar.rs
git commit -m "ui: add sidebar input/action/key/visibility types"
```

### Task 4.2：定义 SidebarLayout 与 SidebarState 骨架

**Files:**
- Modify: `crates/ui/src/sidebar.rs`

- [ ] **Step 1: 加 layout struct**

```rust
#[derive(Debug, Clone)]
pub struct SidebarLayoutItem {
    pub tab_index: usize,
    pub rect: [f32; 4],          // NDC: [left, right, top, bottom]
    pub title: String,
    pub indicator: crate::tab_bar::TabIndicator,
}

#[derive(Debug, Clone, Default)]
pub struct SidebarLayout {
    pub bg_rect: [f32; 4],
    pub header_rect: [f32; 4],
    pub menu_btn_rect: [f32; 4],
    pub new_btn_rect: [f32; 4],
    pub items: Vec<SidebarLayoutItem>,
    pub list_clip: [f32; 4],
    pub settings_btn_rect: [f32; 4],
    pub edge_resize_rect: [f32; 4],
}

#[derive(Default)]
pub struct SidebarState {
    visibility: Visibility,
    layout: Option<SidebarLayout>,
    open_menu: Option<PopupMenu>,
}

impl Default for Visibility { fn default() -> Self { Visibility::Hidden } }

impl SidebarState {
    pub fn new(_cfg: &SidebarConfig) -> Self {
        // 阶段 4 启动即 Pinned，便于联调
        Self { visibility: Visibility::Pinned, ..Self::default() }
    }

    pub fn visibility(&self) -> Visibility { self.visibility }
    pub fn set_visibility(&mut self, v: Visibility) { self.visibility = v; }
    pub fn current_layout(&self) -> Option<&SidebarLayout> { self.layout.as_ref() }
    pub fn open_menu(&self) -> Option<&PopupMenu> { self.open_menu.as_ref() }

    pub fn current_width(&self, cfg: &SidebarConfig) -> f32 {
        match self.visibility {
            Visibility::Hidden => 0.0,
            Visibility::HoverPeek | Visibility::Pinned => cfg.width,
        }
    }

    pub fn editor_left_offset(&self, cfg: &SidebarConfig) -> f32 {
        match self.visibility {
            Visibility::Pinned => cfg.width,
            _ => 0.0,
        }
    }

    pub fn is_visible(&self) -> bool {
        !matches!(self.visibility, Visibility::Hidden)
    }
}
```

- [ ] **Step 2: 加单测覆盖默认状态**

```rust
#[test]
fn sidebar_state_starts_pinned_in_stage4() {
    let cfg = SidebarConfig::new_default(1.0);
    let s = SidebarState::new(&cfg);
    assert_eq!(s.visibility(), Visibility::Pinned);
    assert_eq!(s.current_width(&cfg), 220.0);
    assert_eq!(s.editor_left_offset(&cfg), 220.0);
}

#[test]
fn sidebar_hidden_offsets_zero() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    s.set_visibility(Visibility::Hidden);
    assert_eq!(s.editor_left_offset(&cfg), 0.0);
    assert!(!s.is_visible());
}

#[test]
fn sidebar_hover_peek_does_not_offset_editor() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    s.set_visibility(Visibility::HoverPeek);
    assert_eq!(s.editor_left_offset(&cfg), 0.0);
    assert_eq!(s.current_width(&cfg), 220.0);
    assert!(s.is_visible());
}
```

- [ ] **Step 3: 跑测试**

Run: `cargo test -p edit-plus-ui sidebar`
Expected: PASS。

- [ ] **Step 4: 提交**

```bash
git add crates/ui/src/sidebar.rs
git commit -m "ui: add SidebarState skeleton + width/offset tests"
```

### Task 4.3：实现 update_layout（计算各按钮 / 文件项 NDC 矩形）

**Files:**
- Modify: `crates/ui/src/sidebar.rs`

- [ ] **Step 1: 实现 update_layout**

加：

```rust
use crate::settings::Settings;

const HEADER_H: f32 = 28.0;
const ROW_H: f32 = 24.0;
const NEW_BTN_H: f32 = 28.0;
const SETTINGS_BTN_H: f32 = 28.0;
const PADDING: f32 = 6.0;
const EDGE_RESIZE_W: f32 = 4.0;

impl SidebarState {
    pub fn update_layout(&mut self, input: &SidebarInput<'_>, cfg: &SidebarConfig) {
        if matches!(self.visibility, Visibility::Hidden) {
            self.layout = None;
            return;
        }
        let dpi = Settings::get().dpi_scale;
        let header_h = HEADER_H * dpi;
        let row_h = ROW_H * dpi;
        let new_h = NEW_BTN_H * dpi;
        let settings_h = SETTINGS_BTN_H * dpi;
        let pad = PADDING * dpi;
        let edge_w = EDGE_RESIZE_W * dpi;
        let w = cfg.width;
        let sw = input.screen_w.max(1.0);
        let sh = input.screen_h.max(1.0);

        let to_ndc = |x_px: f32, y_px: f32| -> [f32; 2] {
            [x_px / sw * 2.0 - 1.0, 1.0 - y_px / sh * 2.0]
        };

        let bg = {
            let [l, t] = to_ndc(0.0, 0.0);
            let [r, b] = to_ndc(w, sh);
            [l, r, t, b]
        };
        let header = {
            let [l, t] = to_ndc(0.0, 0.0);
            let [r, b] = to_ndc(w, header_h);
            [l, r, t, b]
        };
        // 红绿灯占左侧 inset.0 像素，☰ 在 header 右侧
        let menu_btn = {
            let menu_x = w - 24.0 * dpi;
            let menu_y = header_h * 0.5 - 8.0 * dpi;
            let [l, t] = to_ndc(menu_x, menu_y);
            let [r, b] = to_ndc(menu_x + 16.0 * dpi, menu_y + 16.0 * dpi);
            [l, r, t, b]
        };
        let new_btn = {
            let y = header_h + pad;
            let [l, t] = to_ndc(pad, y);
            let [r, b] = to_ndc(w - pad, y + new_h);
            [l, r, t, b]
        };
        let list_top_px = header_h + pad + new_h + pad;
        let list_bottom_px = sh - settings_h - pad;
        let mut items = Vec::with_capacity(input.tabs.len());
        for (idx, tab) in input.tabs.iter().enumerate() {
            let item_top = list_top_px + idx as f32 * row_h;
            let item_bottom = item_top + row_h;
            if item_bottom > list_bottom_px { break; } // 暂不分页 / 滚动
            let [l, t] = to_ndc(pad, item_top);
            let [r, b] = to_ndc(w - pad, item_bottom);
            items.push(SidebarLayoutItem {
                tab_index: idx,
                rect: [l, r, t, b],
                title: tab.title.clone(),
                indicator: crate::tab_bar::TabIndicator::for_doc(tab.is_dirty, false),
            });
        }
        let list_clip = {
            let [l, t] = to_ndc(0.0, list_top_px);
            let [r, b] = to_ndc(w, list_bottom_px);
            [l, r, t, b]
        };
        let settings_btn = {
            let y = sh - settings_h;
            let [l, t] = to_ndc(0.0, y);
            let [r, b] = to_ndc(w, sh);
            [l, r, t, b]
        };
        let edge_resize = {
            let [l, t] = to_ndc(w - edge_w, header_h);
            let [r, b] = to_ndc(w + edge_w, sh);
            [l, r, t, b]
        };

        self.layout = Some(SidebarLayout {
            bg_rect: bg,
            header_rect: header,
            menu_btn_rect: menu_btn,
            new_btn_rect: new_btn,
            items,
            list_clip,
            settings_btn_rect: settings_btn,
            edge_resize_rect: edge_resize,
        });
        let _ = input.traffic_light_inset; // 阶段 5 接入
        let _ = input.active_index;        // 渲染时再用
    }
}
```

- [ ] **Step 2: 加 layout 单测**

```rust
#[test]
fn sidebar_layout_zero_items_when_no_tabs() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    let input = SidebarInput {
        tabs: &[],
        active_index: None,
        screen_w: 1200.0,
        screen_h: 800.0,
        traffic_light_inset: (0.0, 0.0),
    };
    s.update_layout(&input, &cfg);
    let layout = s.current_layout().expect("layout populated");
    assert!(layout.items.is_empty());
}

#[test]
fn sidebar_layout_items_match_tab_count() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    let tabs = vec![
        TabInfo { title: "a.rs".into(), file_path: None, is_dirty: false, language: "rust".into() },
        TabInfo { title: "b.rs".into(), file_path: None, is_dirty: true,  language: "rust".into() },
    ];
    let input = SidebarInput {
        tabs: &tabs, active_index: Some(0),
        screen_w: 1200.0, screen_h: 800.0,
        traffic_light_inset: (0.0, 0.0),
    };
    s.update_layout(&input, &cfg);
    assert_eq!(s.current_layout().unwrap().items.len(), 2);
}

#[test]
fn sidebar_layout_none_when_hidden() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    s.set_visibility(Visibility::Hidden);
    let input = SidebarInput {
        tabs: &[], active_index: None,
        screen_w: 1200.0, screen_h: 800.0,
        traffic_light_inset: (0.0, 0.0),
    };
    s.update_layout(&input, &cfg);
    assert!(s.current_layout().is_none());
}
```

- [ ] **Step 3: 跑测试**

Run: `cargo test -p edit-plus-ui sidebar`
Expected: 5/5 PASS（含上一 task 的 3 个）。

- [ ] **Step 4: 提交**

```bash
git add crates/ui/src/sidebar.rs
git commit -m "ui: implement SidebarState::update_layout"
```

### Task 4.4：实现 hit_test 与 on_click

**Files:**
- Modify: `crates/ui/src/sidebar.rs`

- [ ] **Step 1: 加 hit_test 与 on_click**

```rust
fn rect_contains(rect: [f32; 4], ndc_x: f32, ndc_y: f32) -> bool {
    let [l, r, t, b] = rect;
    ndc_x >= l && ndc_x <= r && ndc_y <= t && ndc_y >= b
}

impl SidebarState {
    pub fn on_click(
        &mut self,
        px: f32,
        py: f32,
        button: MouseButton,
        screen_w: f32,
        screen_h: f32,
    ) -> Option<SidebarAction> {
        let layout = self.layout.as_ref()?;
        let ndc_x = px / screen_w * 2.0 - 1.0;
        let ndc_y = 1.0 - py / screen_h * 2.0;

        if rect_contains(layout.menu_btn_rect, ndc_x, ndc_y) {
            return Some(SidebarAction::TogglePin);
        }
        if rect_contains(layout.new_btn_rect, ndc_x, ndc_y) {
            return Some(SidebarAction::NewDocument);
        }
        if rect_contains(layout.settings_btn_rect, ndc_x, ndc_y) {
            return Some(SidebarAction::OpenSettingsMenu);
        }
        for item in &layout.items {
            if rect_contains(item.rect, ndc_x, ndc_y) {
                return match button {
                    MouseButton::Left => Some(SidebarAction::SwitchTab(item.tab_index)),
                    MouseButton::Right => Some(SidebarAction::Context {
                        action: ContextMenuAction::Close, // 占位；右键菜单交给 Context handler 后续展开
                        tab_index: item.tab_index,
                    }),
                };
            }
        }
        None
    }
}
```

- [ ] **Step 2: 加 click 单测**

```rust
#[test]
fn sidebar_click_file_emits_switch_tab() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    let tabs = vec![
        TabInfo { title: "a.rs".into(), file_path: None, is_dirty: false, language: "rust".into() },
    ];
    let sw = 1200.0; let sh = 800.0;
    let input = SidebarInput {
        tabs: &tabs, active_index: Some(0),
        screen_w: sw, screen_h: sh,
        traffic_light_inset: (0.0, 0.0),
    };
    s.update_layout(&input, &cfg);
    let layout = s.current_layout().unwrap();
    let item = &layout.items[0];
    // 取 item 中心，转回像素
    let cx_ndc = (item.rect[0] + item.rect[1]) * 0.5;
    let cy_ndc = (item.rect[2] + item.rect[3]) * 0.5;
    let px = (cx_ndc + 1.0) * 0.5 * sw;
    let py = (1.0 - cy_ndc) * 0.5 * sh;
    let action = s.on_click(px, py, MouseButton::Left, sw, sh);
    assert!(matches!(action, Some(SidebarAction::SwitchTab(0))));
}

#[test]
fn sidebar_click_new_btn_emits_new_doc() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    let input = SidebarInput {
        tabs: &[], active_index: None,
        screen_w: 1200.0, screen_h: 800.0,
        traffic_light_inset: (0.0, 0.0),
    };
    s.update_layout(&input, &cfg);
    let new_rect = s.current_layout().unwrap().new_btn_rect;
    let cx_ndc = (new_rect[0] + new_rect[1]) * 0.5;
    let cy_ndc = (new_rect[2] + new_rect[3]) * 0.5;
    let px = (cx_ndc + 1.0) * 0.5 * 1200.0;
    let py = (1.0 - cy_ndc) * 0.5 * 800.0;
    let action = s.on_click(px, py, MouseButton::Left, 1200.0, 800.0);
    assert!(matches!(action, Some(SidebarAction::NewDocument)));
}

#[test]
fn sidebar_click_outside_returns_none() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    let input = SidebarInput {
        tabs: &[], active_index: None,
        screen_w: 1200.0, screen_h: 800.0,
        traffic_light_inset: (0.0, 0.0),
    };
    s.update_layout(&input, &cfg);
    // 点 (1000, 400) — 超出 220 宽 sidebar
    assert!(s.on_click(1000.0, 400.0, MouseButton::Left, 1200.0, 800.0).is_none());
}
```

- [ ] **Step 3: 跑测试**

Run: `cargo test -p edit-plus-ui sidebar`
Expected: 全 PASS。

- [ ] **Step 4: 提交**

```bash
git add crates/ui/src/sidebar.rs
git commit -m "ui: implement SidebarState hit-test and on_click"
```

### Task 4.5：实现 vertices / text_positions

**Files:**
- Modify: `crates/ui/src/sidebar.rs`

- [ ] **Step 1: 写 vertices**

复用 render::GlyphVertex 与 tab_bar 已有的纯色矩形 helper（如不存在，参考 `tab_bar::tab_bar_vertices` 中纯色矩形的写法 `crates/ui/src/tab_bar.rs:534-862`）。在 sidebar.rs 中加：

```rust
use render::GlyphVertex;

fn fill_quad(rect: [f32; 4], color: [f32; 4]) -> Vec<GlyphVertex> {
    let [l, r, t, b] = rect;
    let v = |x, y| GlyphVertex {
        position: [x, y],
        tex_coord: [0.0, 0.0],
        color,
        is_glyph: 0.0,
    };
    vec![
        v(l, t), v(r, t), v(r, b),
        v(l, t), v(r, b), v(l, b),
    ]
}

impl SidebarState {
    pub fn vertices(&self, _input: &SidebarInput<'_>, theme: &Theme, active_index: Option<usize>) -> Vec<GlyphVertex> {
        let Some(layout) = &self.layout else { return Vec::new(); };
        let mut out = Vec::new();
        out.extend(fill_quad(layout.bg_rect, theme.sidebar_bg));
        out.extend(fill_quad(layout.header_rect, theme.sidebar_header_bg));
        out.extend(fill_quad(layout.new_btn_rect, theme.sidebar_button_bg));
        out.extend(fill_quad(layout.settings_btn_rect, theme.sidebar_header_bg));
        for item in &layout.items {
            let bg = if Some(item.tab_index) == active_index {
                theme.sidebar_item_active_bg
            } else {
                theme.sidebar_item_bg
            };
            out.extend(fill_quad(item.rect, bg));
        }
        out
    }
}
```

`Theme` 中需要新增字段：`sidebar_bg`、`sidebar_header_bg`、`sidebar_button_bg`、`sidebar_item_bg`、`sidebar_item_active_bg`、`sidebar_item_fg`。先在 `crates/ui/src/theme.rs` 加：

```rust
pub sidebar_bg: [f32; 4],
pub sidebar_header_bg: [f32; 4],
pub sidebar_button_bg: [f32; 4],
pub sidebar_item_bg: [f32; 4],
pub sidebar_item_active_bg: [f32; 4],
pub sidebar_item_fg: [f32; 4],
```

light/dark 默认值：

```rust
// dark
sidebar_bg: [0.145, 0.145, 0.149, 1.0],
sidebar_header_bg: [0.118, 0.118, 0.122, 1.0],
sidebar_button_bg: [0.054, 0.388, 0.612, 1.0],
sidebar_item_bg: [0.0, 0.0, 0.0, 0.0], // 透明
sidebar_item_active_bg: [0.216, 0.216, 0.239, 1.0],
sidebar_item_fg: [0.85, 0.85, 0.85, 1.0],

// light
sidebar_bg: [0.95, 0.95, 0.95, 1.0],
sidebar_header_bg: [0.92, 0.92, 0.92, 1.0],
sidebar_button_bg: [0.0, 0.478, 1.0, 1.0],
sidebar_item_bg: [0.0, 0.0, 0.0, 0.0],
sidebar_item_active_bg: [0.85, 0.85, 0.85, 1.0],
sidebar_item_fg: [0.15, 0.15, 0.15, 1.0],
```

- [ ] **Step 2: 写 text_positions**

```rust
#[derive(Debug, Clone)]
pub struct SidebarText {
    pub text: String,
    pub x_px: f32,
    pub y_px: f32,
    pub color: [f32; 4],
}

impl SidebarState {
    pub fn text_positions(&self, screen_w: f32, screen_h: f32, theme: &Theme, font_size: f32) -> Vec<SidebarText> {
        let Some(layout) = &self.layout else { return Vec::new(); };
        let mut out = Vec::new();
        let ndc_to_px = |ndc_x: f32, ndc_y: f32| ->(f32,f32) {
            ((ndc_x + 1.0) * 0.5 * screen_w, (1.0 - ndc_y) * 0.5 * screen_h)
        };
        let pad = 8.0 * Settings::get().dpi_scale;
        // 新建按钮文字
        {
            let cy_ndc = (layout.new_btn_rect[2] + layout.new_btn_rect[3]) * 0.5;
            let (_, py) = ndc_to_px(layout.new_btn_rect[0], cy_ndc);
            let (px, _) = ndc_to_px(layout.new_btn_rect[0], 0.0);
            out.push(SidebarText {
                text: "+ 新建".into(),
                x_px: px + pad,
                y_px: py + font_size * 0.35,
                color: theme.sidebar_item_fg,
            });
        }
        // 设置按钮文字
        {
            let cy_ndc = (layout.settings_btn_rect[2] + layout.settings_btn_rect[3]) * 0.5;
            let (_, py) = ndc_to_px(layout.settings_btn_rect[0], cy_ndc);
            let (px, _) = ndc_to_px(layout.settings_btn_rect[0], 0.0);
            out.push(SidebarText {
                text: "⚙ 设置".into(),
                x_px: px + pad,
                y_px: py + font_size * 0.35,
                color: theme.sidebar_item_fg,
            });
        }
        // 文件项标题
        for item in &layout.items {
            let cy_ndc = (item.rect[2] + item.rect[3]) * 0.5;
            let (_, py) = ndc_to_px(item.rect[0], cy_ndc);
            let (px, _) = ndc_to_px(item.rect[0], 0.0);
            let label = if matches!(item.indicator, crate::tab_bar::TabIndicator::Dirty) {
                format!("● {}", item.title)
            } else {
                item.title.clone()
            };
            out.push(SidebarText {
                text: label,
                x_px: px + pad,
                y_px: py + font_size * 0.35,
                color: theme.sidebar_item_fg,
            });
        }
        out
    }
}
```

- [ ] **Step 3: 跑 ui 编译 / 测试**

Run: `cargo test -p edit-plus-ui sidebar`
Expected: PASS。

- [ ] **Step 4: 提交**

```bash
git add crates/ui/src/sidebar.rs crates/ui/src/theme.rs
git commit -m "ui: render sidebar background/items + text positions"
```

### Task 4.6：app 接入 sidebar 渲染

**Files:**
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/events.rs`

- [ ] **Step 1: workspace 持有 sidebar_state**

```rust
use ui::sidebar::SidebarState;

pub(crate) struct Workspace {
    // 已有
    pub(crate) sidebar_state: SidebarState,
}

impl Workspace {
    pub(crate) fn new() -> Self {
        let dpi = Settings::get_static().dpi_scale;
        let sidebar_cfg = SidebarConfig::new_default(dpi);
        Self {
            // ...
            sidebar_state: SidebarState::new(&sidebar_cfg),
            sidebar_cfg,
        }
    }
}
```

- [ ] **Step 2: app_renderer 顶层按 view_mode 分支**

定位 `crates/app/src/app_renderer.rs` 中 tab_bar 渲染入口（约 :458-：582）。包一层：

```rust
match Settings::get_static().view_mode {
    ui::view_mode::ViewMode::Tabs => {
        // 既有 tab bar 渲染
    }
    ui::view_mode::ViewMode::Sidebar => {
        let tab_infos: Vec<ui::tab_bar::TabInfo> = self.workspace.doc_views.iter()
            .map(|dv| /* 同 tab 路径中的构造 */).collect();
        let input = ui::sidebar::SidebarInput {
            tabs: &tab_infos,
            active_index: Some(self.workspace.active_index),
            screen_w, screen_h,
            traffic_light_inset: (0.0, 0.0), // 阶段 5 替换
        };
        self.workspace.sidebar_state.update_layout(&input, &self.workspace.sidebar_cfg);
        let verts = self.workspace.sidebar_state.vertices(&input, &self.current_theme,
            Some(self.workspace.active_index));
        // append verts 到 quad pass；text 走 cosmic-text path（参考 status_bar 的 text positions 集成）
    }
}
```

文本渲染入口同样按 view_mode 分支：sidebar 模式下用 `sidebar_state.text_positions(...)` 返回的列表，调用既有 cosmic-text 文字 vertices 流水线（参照 `app_renderer.rs` 里 status_bar / tab_bar 文本路径的写法）。

- [ ] **Step 3: 编辑区左边距整合 sidebar 让位**

定位编辑区视口 left 计算的位置（搜索 `content_left_margin` 或 `gutter_width` 的调用）。在 sidebar 模式下加上 `sidebar_state.editor_left_offset(&cfg)` + `Settings::dpi_scale * 10.0`：

```rust
let sidebar_left = match Settings::get_static().view_mode {
    ui::view_mode::ViewMode::Sidebar => self.workspace.sidebar_state.editor_left_offset(&self.workspace.sidebar_cfg),
    ui::view_mode::ViewMode::Tabs => 0.0,
};
let editor_left = sidebar_left + Settings::get_static().dpi_scale * 10.0;
```

`editor_left` 加到 viewport / cursor / gutter 的水平偏移上。

- [ ] **Step 4: events 路径接 sidebar 点击**

`crates/app/src/events.rs` 里 mouse_click handler 顶部分支：

```rust
match Settings::get_static().view_mode {
    ui::view_mode::ViewMode::Tabs => {
        // 既有 tab_bar hit / click 处理
    }
    ui::view_mode::ViewMode::Sidebar => {
        if let Some(action) = workspace.sidebar_state.on_click(
            px, py,
            if right_button { ui::sidebar::MouseButton::Right } else { ui::sidebar::MouseButton::Left },
            screen_w, screen_h,
        ) {
            return apply_sidebar_action(workspace, action, /* AppAction sink */);
        }
    }
}
```

新增 helper `fn apply_sidebar_action(...)`：

```rust
fn apply_sidebar_action(
    workspace: &mut Workspace,
    action: ui::sidebar::SidebarAction,
    actions: &mut Vec<AppAction>,
) {
    use ui::sidebar::SidebarAction as SA;
    match action {
        SA::SwitchTab(idx) => actions.push(AppAction::SwitchTab(idx)),
        SA::NewDocument   => actions.push(AppAction::NewEmptyTab),
        SA::OpenSettingsMenu => { /* 阶段 8 */ }
        SA::ToggleViewMode   => { /* 阶段 8 */ }
        SA::TogglePin        => { /* 阶段 6 / 8 */ }
        SA::SetWidth(_)      => { /* 阶段 7 */ }
        SA::Context { .. }   => { /* 阶段 9 */ }
    }
}
```

- [ ] **Step 5: 把 ViewMode::default() 改回 Sidebar**

修改 `crates/ui/src/view_mode.rs`：

```rust
impl Default for ViewMode {
    fn default() -> Self { ViewMode::Sidebar }
}
```

更新 `settings_io::tests::default_view_mode_is_tabs` 改为 `default_view_mode_is_sidebar`，断言 `ViewMode::Sidebar`；更新 `crates/ui/src/settings.rs` 的 default_settings 测试。

- [ ] **Step 6: 跑全测**

Run: `cargo test -p edit-plus-ui -p edit-plus-app`
Expected: 全 PASS。

- [ ] **Step 7: 启动手测**

Run: `cargo run -p edit-plus-app -- assets/samples/medium_ascii_5mb.txt`
预期：
- 默认进入 sidebar 模式（红绿灯仍在原生位，因阶段 5 还没接 titlebar）
- 左侧 220px sidebar 显示文件列表
- 点击文件项切换；编辑区水平让位 220px
- 没有 Tab 栏

- [ ] **Step 8: 提交**

```bash
git add crates/app/src/workspace.rs crates/app/src/app_renderer.rs crates/app/src/events.rs crates/ui/src/view_mode.rs crates/ui/src/settings.rs crates/app/src/settings_io.rs
git commit -m "app: wire sidebar render + click into render pipeline (Pinned only)"
```

---

# 阶段 5：macOS NSWindow titlebar 桥接

**Goal**: 在 sidebar 模式下，把红绿灯按钮浮到 sidebar header 上方，编辑区上面无独立 titlebar；切回 Tabs 时还原。

### Task 5.1：sys 模块骨架

**Files:**
- Create: `crates/app/src/sys/mod.rs`
- Create: `crates/app/src/sys/macos_titlebar.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: 创建 sys 模块**

```rust
// crates/app/src/sys/mod.rs
pub(crate) mod macos_titlebar;
```

```rust
// crates/app/src/sys/macos_titlebar.rs

#[cfg(target_os = "macos")]
mod imp {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSWindow, NSWindowStyleMask, NSWindowTitleVisibility, NSWindowButton};
    use objc2_foundation::CGFloat;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
        let handle = window.window_handle().ok()?.as_raw();
        let RawWindowHandle::AppKit(h) = handle else { return None; };
        // h.ns_view 是 NSView 指针；NSView.window 给我们 NSWindow
        unsafe {
            let ns_view: *mut AnyObject = h.ns_view.as_ptr() as *mut _;
            let ns_window_ptr: *mut NSWindow = objc2::msg_send![ns_view, window];
            if ns_window_ptr.is_null() { return None; }
            Some(Retained::retain(ns_window_ptr).unwrap())
        }
    }

    pub fn enable_full_size_content(window: &Window) {
        let Some(ns_window) = ns_window(window) else { return; };
        unsafe {
            ns_window.setTitlebarAppearsTransparent(true);
            ns_window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
            let mut mask = ns_window.styleMask();
            mask |= NSWindowStyleMask::FullSizeContentView;
            ns_window.setStyleMask(mask);
        }
    }

    pub fn disable_full_size_content(window: &Window) {
        let Some(ns_window) = ns_window(window) else { return; };
        unsafe {
            ns_window.setTitlebarAppearsTransparent(false);
            ns_window.setTitleVisibility(NSWindowTitleVisibility::Visible);
            let mut mask = ns_window.styleMask();
            mask &= !NSWindowStyleMask::FullSizeContentView;
            ns_window.setStyleMask(mask);
        }
    }

    pub fn traffic_light_inset(window: &Window) -> (f32, f32) {
        let Some(ns_window) = ns_window(window) else { return (0.0, 0.0); };
        unsafe {
            let close_btn = ns_window.standardWindowButton(NSWindowButton::CloseButton);
            let Some(btn) = close_btn else { return (0.0, 0.0); };
            let frame = btn.frame();
            // frame.origin.x 是按钮左侧 inset；从按钮右沿到内容左沿大约再加 8px
            let left = (frame.origin.x as f32) + (frame.size.width as f32) * 3.5; // 3 个按钮 + 间距
            let top = (frame.origin.y as f32) + (frame.size.height as f32) + 4.0;
            (left, top)
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use winit::window::Window;
    pub fn enable_full_size_content(_w: &Window) {}
    pub fn disable_full_size_content(_w: &Window) {}
    pub fn traffic_light_inset(_w: &Window) -> (f32, f32) { (0.0, 0.0) }
}

pub(crate) use imp::{disable_full_size_content, enable_full_size_content, traffic_light_inset};
```

- [ ] **Step 2: app/src/lib.rs 注册 mod sys**

加 `mod sys;` 到 lib.rs。

- [ ] **Step 3: 编译**

Run: `cargo build -p edit-plus-app`
Expected: 0 错误。如有 objc2 API 签名差异，调整 `NSWindowButton::CloseButton` / `setStyleMask` 的具体路径，参考 objc2-app-kit 文档。

- [ ] **Step 4: 提交**

```bash
git add crates/app/src/sys crates/app/src/lib.rs
git commit -m "app: add macos_titlebar bridge for NSWindow fullSizeContentView"
```

### Task 5.2：在 App 启动 / 模式切换时调用

**Files:**
- Modify: `crates/app/src/app.rs`

- [ ] **Step 1: 启动时根据 view_mode 调用**

定位 App 创建 winit window 之后的位置（搜索 `create_window` 或 `init_window`）。加：

```rust
match Settings::get_static().view_mode {
    ui::view_mode::ViewMode::Sidebar => crate::sys::macos_titlebar::enable_full_size_content(&window),
    ui::view_mode::ViewMode::Tabs => {}
}
```

- [ ] **Step 2: 把 traffic_light_inset 注入 sidebar input**

在 `app_renderer.rs` 构造 `SidebarInput` 时把 `traffic_light_inset` 改为：

```rust
traffic_light_inset: crate::sys::macos_titlebar::traffic_light_inset(&self.window),
```

`self.window` 需要在 App 上有引用；如已存在则直接用，否则把 Window 引用透传到 renderer 的对应方法。

- [ ] **Step 3: sidebar 内部用 inset**

修改 `crates/ui/src/sidebar.rs::update_layout` 的 menu_btn 位置：

```rust
let header_left_pad = input.traffic_light_inset.0; // 红绿灯占位
let menu_btn = {
    let menu_x = w - 24.0 * dpi;
    let menu_y = header_h * 0.5 - 8.0 * dpi;
    // ...
};
// header text / menu_btn / new_btn 顶部不能低于 traffic_light_inset.1
```

确保 header 高度 ≥ `input.traffic_light_inset.1` 或在 ☰ 按钮渲染时让位 `traffic_light_inset.0`。

更新对应的 sidebar 单测，加：

```rust
#[test]
fn sidebar_layout_respects_traffic_light_inset() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    let input = SidebarInput {
        tabs: &[], active_index: None,
        screen_w: 1200.0, screen_h: 800.0,
        traffic_light_inset: (78.0, 28.0),
    };
    s.update_layout(&input, &cfg);
    let layout = s.current_layout().unwrap();
    // header 高度至少覆盖 inset.1
    let header_top_ndc = layout.header_rect[2];
    let header_bottom_ndc = layout.header_rect[3];
    let header_h_ndc = header_top_ndc - header_bottom_ndc;
    let header_h_px = header_h_ndc * 0.5 * 800.0;
    assert!(header_h_px + 0.5 >= 28.0);
}
```

- [ ] **Step 4: 跑测试 + 启动**

Run: `cargo test -p edit-plus-ui -p edit-plus-app`
Expected: 全 PASS。

Run: `cargo run -p edit-plus-app -- assets/samples/medium_ascii_5mb.txt`
预期（macOS）：
- 红绿灯按钮浮在窗口左上角，编辑区无独立 titlebar
- ☰ 在 sidebar header 右侧
- 切到 ViewMode::Tabs（手动改 settings.yaml）后红绿灯回原生位

- [ ] **Step 5: 提交**

```bash
git add crates/app/src/app.rs crates/app/src/app_renderer.rs crates/ui/src/sidebar.rs
git commit -m "app: enable fullSizeContentView when sidebar mode active"
```

### Task 5.3：模式切换时调整 NSWindow

**Files:**
- Modify: `crates/app/src/app.rs`

- [ ] **Step 1: 加切换函数**

```rust
impl App {
    fn apply_view_mode(&self, view_mode: ui::view_mode::ViewMode) {
        match view_mode {
            ui::view_mode::ViewMode::Sidebar => crate::sys::macos_titlebar::enable_full_size_content(&self.window),
            ui::view_mode::ViewMode::Tabs    => crate::sys::macos_titlebar::disable_full_size_content(&self.window),
        }
    }
}
```

阶段 8 设置菜单切模式时调用此函数；当前 stage 5 仅启动调一次，先放 stub 接口。

- [ ] **Step 2: 提交**

```bash
git add crates/app/src/app.rs
git commit -m "app: expose apply_view_mode for runtime mode switches"
```

---

# 阶段 6：hover 状态机

**Goal**: 实现 4px 热区进入 + 150ms 延时 → HoverPeek；离开 sidebar 区 + 300ms → Hidden；按 Esc 立即收起；Cmd+B 切 Pinned ↔ Hidden。

### Task 6.1：在 SidebarState 加计时字段与 tick

**Files:**
- Modify: `crates/ui/src/sidebar.rs`

- [ ] **Step 1: 加字段与方法**

```rust
use std::time::{Duration, Instant};

const HOTZONE_W_PX: f32 = 4.0;
const HOVER_ENTER_MS: u64 = 150;
const HOVER_LEAVE_MS: u64 = 300;

pub struct SidebarState {
    visibility: Visibility,
    hover_enter_at: Option<Instant>,
    hover_leave_at: Option<Instant>,
    layout: Option<SidebarLayout>,
    open_menu: Option<PopupMenu>,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self {
            visibility: Visibility::Hidden,
            hover_enter_at: None,
            hover_leave_at: None,
            layout: None,
            open_menu: None,
        }
    }
}
```

把 `SidebarState::new` 改为以 `cfg.pinned` 决定初始状态：

```rust
pub fn new(cfg: &SidebarConfig) -> Self {
    Self {
        visibility: if cfg.pinned { Visibility::Pinned } else { Visibility::Hidden },
        ..Self::default()
    }
}
```

加方法：

```rust
impl SidebarState {
    pub fn on_mouse_move(&mut self, px: f32, _py: f32, screen_w: f32, _screen_h: f32, cfg: &SidebarConfig) {
        if matches!(self.visibility, Visibility::Pinned) { return; }
        let in_hot_zone = px <= HOTZONE_W_PX * Settings::get().dpi_scale;
        let in_sidebar = px <= cfg.width;
        match self.visibility {
            Visibility::Hidden => {
                if in_hot_zone {
                    if self.hover_enter_at.is_none() {
                        self.hover_enter_at = Some(Instant::now());
                    }
                } else {
                    self.hover_enter_at = None;
                }
            }
            Visibility::HoverPeek => {
                if !in_sidebar {
                    if self.hover_leave_at.is_none() {
                        self.hover_leave_at = Some(Instant::now());
                    }
                } else {
                    self.hover_leave_at = None;
                }
            }
            Visibility::Pinned => {}
        }
        let _ = screen_w;
    }

    pub fn on_mouse_leave(&mut self) {
        if matches!(self.visibility, Visibility::HoverPeek) && self.hover_leave_at.is_none() {
            self.hover_leave_at = Some(Instant::now());
        }
        self.hover_enter_at = None;
    }

    pub fn tick(&mut self, now: Instant) {
        match self.visibility {
            Visibility::Hidden => {
                if let Some(t) = self.hover_enter_at {
                    if now.duration_since(t) >= Duration::from_millis(HOVER_ENTER_MS) {
                        self.visibility = Visibility::HoverPeek;
                        self.hover_enter_at = None;
                    }
                }
            }
            Visibility::HoverPeek => {
                if let Some(t) = self.hover_leave_at {
                    if now.duration_since(t) >= Duration::from_millis(HOVER_LEAVE_MS) {
                        self.visibility = Visibility::Hidden;
                        self.hover_leave_at = None;
                    }
                }
            }
            Visibility::Pinned => {}
        }
    }

    pub fn on_key(&mut self, key: SidebarKey, cfg: &mut SidebarConfig) -> Option<SidebarAction> {
        match key {
            SidebarKey::TogglePin => {
                self.visibility = match self.visibility {
                    Visibility::Pinned => Visibility::Hidden,
                    _ => Visibility::Pinned,
                };
                cfg.pinned = matches!(self.visibility, Visibility::Pinned);
                Some(SidebarAction::TogglePin)
            }
            SidebarKey::Escape => {
                if matches!(self.visibility, Visibility::HoverPeek) {
                    self.visibility = Visibility::Hidden;
                    self.hover_leave_at = None;
                    Some(SidebarAction::TogglePin) // 触发持久化（保持当前 pinned 状态）
                } else {
                    None
                }
            }
        }
    }
}
```

- [ ] **Step 2: 单测覆盖**

```rust
#[test]
fn sidebar_hover_enter_after_150ms() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    assert_eq!(s.visibility(), Visibility::Hidden);
    let t0 = Instant::now();
    s.on_mouse_move(2.0, 100.0, 1200.0, 800.0, &cfg);
    s.tick(t0 + Duration::from_millis(100));
    assert_eq!(s.visibility(), Visibility::Hidden);
    s.tick(t0 + Duration::from_millis(160));
    assert_eq!(s.visibility(), Visibility::HoverPeek);
}

#[test]
fn sidebar_hover_exit_after_300ms() {
    let mut cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    s.set_visibility(Visibility::HoverPeek);
    let t0 = Instant::now();
    s.on_mouse_move(500.0, 100.0, 1200.0, 800.0, &cfg);
    s.tick(t0 + Duration::from_millis(100));
    assert_eq!(s.visibility(), Visibility::HoverPeek);
    s.tick(t0 + Duration::from_millis(310));
    assert_eq!(s.visibility(), Visibility::Hidden);
    let _ = cfg;
}

#[test]
fn sidebar_pinned_immune_to_hover_leave() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    s.set_visibility(Visibility::Pinned);
    s.on_mouse_move(900.0, 100.0, 1200.0, 800.0, &cfg);
    s.tick(Instant::now() + Duration::from_secs(5));
    assert_eq!(s.visibility(), Visibility::Pinned);
}

#[test]
fn sidebar_cmdb_toggles_pin() {
    let mut cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    let action = s.on_key(SidebarKey::TogglePin, &mut cfg);
    assert!(matches!(action, Some(SidebarAction::TogglePin)));
    assert_eq!(s.visibility(), Visibility::Pinned);
    assert!(cfg.pinned);
    s.on_key(SidebarKey::TogglePin, &mut cfg);
    assert_eq!(s.visibility(), Visibility::Hidden);
    assert!(!cfg.pinned);
}

#[test]
fn sidebar_esc_collapses_hover_only() {
    let mut cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    s.set_visibility(Visibility::HoverPeek);
    s.on_key(SidebarKey::Escape, &mut cfg);
    assert_eq!(s.visibility(), Visibility::Hidden);
    // Pinned 时 Esc 无效
    s.set_visibility(Visibility::Pinned);
    s.on_key(SidebarKey::Escape, &mut cfg);
    assert_eq!(s.visibility(), Visibility::Pinned);
}
```

- [ ] **Step 3: 跑测试**

Run: `cargo test -p edit-plus-ui sidebar`
Expected: 全 PASS。

- [ ] **Step 4: 提交**

```bash
git add crates/ui/src/sidebar.rs
git commit -m "ui: add sidebar hover state machine + Cmd+B/Esc keys"
```

### Task 6.2：app 把鼠标位置 / Cmd+B / Esc / tick 转发进来

**Files:**
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/input.rs`
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_renderer.rs`

- [ ] **Step 1: events.rs 转发 mouse_move**

在 mouse_move handler 中加：

```rust
if matches!(Settings::get_static().view_mode, ui::view_mode::ViewMode::Sidebar) {
    workspace.sidebar_state.on_mouse_move(px, py, screen_w, screen_h, &workspace.sidebar_cfg);
}
```

- [ ] **Step 2: input.rs / 快捷键处理：Cmd+B 与 Esc**

定位现有键盘处理（搜索 `KeyCode::Escape` 或 `KeyCode::KeyB`）。在 modifier == Super 且 key == B 的分支加：

```rust
if matches!(Settings::get_static().view_mode, ui::view_mode::ViewMode::Sidebar) {
    if let Some(action) = workspace.sidebar_state.on_key(ui::sidebar::SidebarKey::TogglePin, &mut workspace.sidebar_cfg) {
        apply_sidebar_action(workspace, action, &mut actions);
    }
    return;
}
```

Esc 同理，分支：仅当 sidebar 处于 HoverPeek 时拦截 Esc，否则继续走原 Esc 逻辑（关闭 search bar 等）。

- [ ] **Step 3: 主循环每帧 tick**

定位 `event_loop.run` 或 `Window::request_redraw` 之前的位置，加：

```rust
if matches!(Settings::get_static().view_mode, ui::view_mode::ViewMode::Sidebar) {
    self.workspace.sidebar_state.tick(std::time::Instant::now());
}
```

并在 `tick` 后若 `sidebar_state.visibility()` 变化（hover 进入 / 离开），调用 `self.window.request_redraw()`。可在 `SidebarState` 加一个 `pub fn tick_returns_changed(...)` 返回 bool 表示状态变化。简化：每帧都 redraw；阶段 6 优先正确性。

- [ ] **Step 4: TogglePin → 写盘**

`apply_sidebar_action` 中：

```rust
SA::TogglePin => {
    workspace.save_snapshot();
}
```

- [ ] **Step 5: 跑测试 + 启动手测**

Run: `cargo test -p edit-plus-app`
Run: `cargo run -p edit-plus-app -- assets/samples/medium_ascii_5mb.txt`

预期：
- 启动 sidebar 隐藏；鼠标贴左 < 4px 停 150ms 后弹出 overlay
- 鼠标离开 sidebar 区 300ms 后消失
- Cmd+B 钉住，再按取消钉住
- 钉住状态下重启 app 仍钉住（来自 workspace.yaml 持久化）
- Esc 仅在 hover overlay 状态下收起

- [ ] **Step 6: 提交**

```bash
git add crates/app/src/events.rs crates/app/src/input.rs crates/app/src/app.rs crates/app/src/app_renderer.rs
git commit -m "app: forward mouse/Cmd+B/Esc to sidebar + tick each frame"
```

---

# 阶段 7：边缘拖拽改宽 + 持久化

**Goal**: 在 sidebar 右边缘 4px 拖拽热区改宽；范围 `[160 * dpi, 400 * dpi]`；只在 `on_drag_end` 写盘。

### Task 7.1：SidebarState 加 EdgeDragState 与 on_drag

**Files:**
- Modify: `crates/ui/src/sidebar.rs`

- [ ] **Step 1: 加字段**

```rust
struct EdgeDragState {
    start_px: f32,
    start_width: f32,
}

pub struct SidebarState {
    visibility: Visibility,
    hover_enter_at: Option<Instant>,
    hover_leave_at: Option<Instant>,
    drag: Option<EdgeDragState>,
    layout: Option<SidebarLayout>,
    open_menu: Option<PopupMenu>,
}
```

补回 `Default` impl 把 `drag: None` 加入。

- [ ] **Step 2: 加 on_drag_start / on_drag / on_drag_end**

```rust
impl SidebarState {
    pub fn on_drag_start(&mut self, px: f32, _py: f32, cfg: &SidebarConfig, screen_w: f32) -> bool {
        if !self.is_visible() { return false; }
        let edge = cfg.width;
        let band = 4.0 * Settings::get().dpi_scale;
        if (px - edge).abs() <= band {
            self.drag = Some(EdgeDragState { start_px: px, start_width: cfg.width });
            true
        } else {
            let _ = screen_w;
            false
        }
    }

    pub fn on_drag(&mut self, px: f32, _py: f32, cfg: &mut SidebarConfig) -> Option<SidebarAction> {
        let drag = self.drag.as_ref()?;
        let dpi = Settings::get().dpi_scale;
        let mut new_w = drag.start_width + (px - drag.start_px);
        new_w = new_w.clamp(160.0 * dpi, 400.0 * dpi);
        cfg.width = new_w;
        Some(SidebarAction::SetWidth(new_w))
    }

    pub fn on_drag_end(&mut self) -> Option<SidebarAction> {
        let drag = self.drag.take()?;
        let _ = drag;
        Some(SidebarAction::TogglePin) // 触发 workspace.save_snapshot；阶段 8 后改为专门的 PersistConfig 信号
    }
}
```

> 注：`SidebarAction::TogglePin` 仅作为占位通知 app 写盘；如担心混淆，加新变体 `PersistConfig`：

```rust
pub enum SidebarAction {
    // ... 既有
    PersistConfig,
}
```

并把 `on_drag_end` 改为 `Some(SidebarAction::PersistConfig)`。app 层的 `apply_sidebar_action` 增加：

```rust
SA::PersistConfig => workspace.save_snapshot(),
SA::SetWidth(w)   => workspace.sidebar_cfg.width = w,
```

- [ ] **Step 3: 单测**

```rust
#[test]
fn sidebar_width_drag_clamp() {
    let mut cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    s.set_visibility(Visibility::Pinned);
    s.on_drag_start(220.0, 100.0, &cfg, 1200.0);
    // 拖到 50 像素 → 应被 clamp 到 160
    let action = s.on_drag(50.0, 100.0, &mut cfg);
    assert!(matches!(action, Some(SidebarAction::SetWidth(_))));
    assert_eq!(cfg.width, 160.0);
    // 拖到 9999 → clamp 到 400
    s.on_drag(9999.0, 100.0, &mut cfg);
    assert_eq!(cfg.width, 400.0);
}

#[test]
fn sidebar_width_drag_only_persists_on_drag_end() {
    let mut cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    s.set_visibility(Visibility::Pinned);
    assert!(s.on_drag_start(220.0, 100.0, &cfg, 1200.0));
    let mid = s.on_drag(300.0, 100.0, &mut cfg);
    assert!(matches!(mid, Some(SidebarAction::SetWidth(_))));
    let end = s.on_drag_end();
    assert!(matches!(end, Some(SidebarAction::PersistConfig)));
}

#[test]
fn sidebar_drag_start_outside_band_returns_false() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    s.set_visibility(Visibility::Pinned);
    assert!(!s.on_drag_start(50.0, 100.0, &cfg, 1200.0));
}
```

- [ ] **Step 4: 跑测试**

Run: `cargo test -p edit-plus-ui sidebar`
Expected: 全 PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/ui/src/sidebar.rs
git commit -m "ui: implement sidebar edge drag resize with clamp"
```

### Task 7.2：app 接入拖拽

**Files:**
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/mouse.rs`

- [ ] **Step 1: 在 mouse_down handler 中尝试 sidebar drag start**

```rust
if matches!(Settings::get_static().view_mode, ui::view_mode::ViewMode::Sidebar) {
    if workspace.sidebar_state.on_drag_start(px, py, &workspace.sidebar_cfg, screen_w) {
        workspace.dragging_sidebar = true;
        return; // 不再走点击逻辑
    }
}
```

`Workspace` 加字段 `pub(crate) dragging_sidebar: bool` 默认 false。

- [ ] **Step 2: mouse_move handler 中拖拽分支**

```rust
if workspace.dragging_sidebar {
    if let Some(action) = workspace.sidebar_state.on_drag(px, py, &mut workspace.sidebar_cfg) {
        apply_sidebar_action(workspace, action, &mut actions);
    }
    return;
}
```

- [ ] **Step 3: mouse_up 提交拖拽**

```rust
if workspace.dragging_sidebar {
    workspace.dragging_sidebar = false;
    if let Some(action) = workspace.sidebar_state.on_drag_end() {
        apply_sidebar_action(workspace, action, &mut actions);
    }
    return;
}
```

- [ ] **Step 4: 启动手测**

Run: `cargo run -p edit-plus-app -- assets/samples/medium_ascii_5mb.txt`
预期：
- 钉住 sidebar，鼠标移到右边缘出现 ↔ resize 光标
- 拖拽改宽，松开后重启 app 宽度保持
- 拖到极限 clamp 到 [160, 400]

- [ ] **Step 5: 提交**

```bash
git add crates/app/src/events.rs crates/app/src/mouse.rs crates/app/src/workspace.rs
git commit -m "app: route sidebar edge drag through mouse handlers"
```

---

# 阶段 8：设置菜单

**Goal**: 点击设置按钮弹出 PopupMenu，包含「Sidebar 模式 ✓ / Tabs 模式」「打开 settings.yaml」三个项；选择切换模式时立即写盘 + 调 `apply_view_mode`；选择「打开 settings.yaml」走 workspace 打开新 tab。

### Task 8.1：SidebarState::open_settings_menu 构造菜单

**Files:**
- Modify: `crates/ui/src/sidebar.rs`
- Modify: `crates/ui/src/popup_menu.rs`

- [ ] **Step 1: 定义设置菜单项的 Custom action 编码**

在 `crates/ui/src/sidebar.rs` 顶部加常量：

```rust
pub mod settings_action {
    pub const VIEW_MODE_SIDEBAR: u32 = 1;
    pub const VIEW_MODE_TABS: u32 = 2;
    pub const OPEN_SETTINGS_FILE: u32 = 3;
}
```

- [ ] **Step 2: 加 open_settings_menu 方法**

```rust
impl SidebarState {
    pub fn open_settings_menu(
        &mut self,
        current_mode: crate::view_mode::ViewMode,
        screen_w: f32,
        screen_h: f32,
    ) {
        let layout = match self.layout.as_ref() { Some(l) => l, None => return };
        let anchor_x_ndc = layout.settings_btn_rect[1] + 0.01;
        let anchor_y_ndc = layout.settings_btn_rect[2];
        let items = vec![
            PopupMenuItem {
                label: "Sidebar 模式".into(),
                is_active: matches!(current_mode, crate::view_mode::ViewMode::Sidebar),
                is_separator: false,
                action: PopupMenuAction::Custom(settings_action::VIEW_MODE_SIDEBAR),
            },
            PopupMenuItem {
                label: "Tabs 模式".into(),
                is_active: matches!(current_mode, crate::view_mode::ViewMode::Tabs),
                is_separator: false,
                action: PopupMenuAction::Custom(settings_action::VIEW_MODE_TABS),
            },
            PopupMenuItem { label: "".into(), is_active: false, is_separator: true, action: PopupMenuAction::Custom(0) },
            PopupMenuItem {
                label: "打开 settings.yaml".into(),
                is_active: false, is_separator: false,
                action: PopupMenuAction::Custom(settings_action::OPEN_SETTINGS_FILE),
            },
        ];
        let menu = PopupMenu::for_items(items, [anchor_x_ndc, anchor_y_ndc], screen_w, screen_h);
        self.open_menu = Some(menu);
    }

    pub fn dispatch_menu_click(&mut self, ndc_x: f32, ndc_y: f32) -> Option<SidebarAction> {
        let menu = self.open_menu.as_ref()?;
        let action = *menu.hit_test(ndc_x, ndc_y)?;
        self.open_menu = None;
        match action {
            PopupMenuAction::Custom(id) if id == settings_action::VIEW_MODE_SIDEBAR
                => Some(SidebarAction::ToggleViewMode),
            PopupMenuAction::Custom(id) if id == settings_action::VIEW_MODE_TABS
                => Some(SidebarAction::ToggleViewMode),
            PopupMenuAction::Custom(id) if id == settings_action::OPEN_SETTINGS_FILE
                => Some(SidebarAction::NewDocument), // 占位，由 app 层把 SidebarAction::NewDocument 路径升级为打开 settings 文件
            _ => None,
        }
    }
}
```

为避免 `NewDocument` 被滥用，扩展 `SidebarAction`：

```rust
pub enum SidebarAction {
    // 已有
    OpenSettingsFile,
    SetViewMode(crate::view_mode::ViewMode),
}
```

把 `dispatch_menu_click` 中的占位换成对应变体：

```rust
PopupMenuAction::Custom(id) if id == settings_action::VIEW_MODE_SIDEBAR
    => Some(SidebarAction::SetViewMode(crate::view_mode::ViewMode::Sidebar)),
PopupMenuAction::Custom(id) if id == settings_action::VIEW_MODE_TABS
    => Some(SidebarAction::SetViewMode(crate::view_mode::ViewMode::Tabs)),
PopupMenuAction::Custom(id) if id == settings_action::OPEN_SETTINGS_FILE
    => Some(SidebarAction::OpenSettingsFile),
```

- [ ] **Step 3: PopupMenu::for_items 工厂**

在 `crates/ui/src/popup_menu.rs` 加：

```rust
impl PopupMenu {
    pub fn for_items(
        items: Vec<PopupMenuItem>,
        anchor_ndc: [f32; 2],
        screen_w: f32,
        screen_h: f32,
    ) -> Self {
        let dpi = crate::settings::Settings::get().dpi_scale;
        let row_h_ndc = (24.0 * dpi) / screen_h * 2.0;
        let sep_h_ndc = (8.0 * dpi) / screen_h * 2.0;
        let menu_w_ndc = (180.0 * dpi) / screen_w * 2.0;
        let menu_left = anchor_ndc[0];
        let menu_right = (menu_left + menu_w_ndc).min(1.0);
        let mut top = anchor_ndc[1];
        let mut item_rects = Vec::with_capacity(items.len());
        for it in &items {
            let h = if it.is_separator { sep_h_ndc } else { row_h_ndc };
            let bottom = top - h;
            item_rects.push([menu_left, menu_right, top, bottom]);
            top = bottom;
        }
        let menu_bottom = top.max(-1.0);
        PopupMenu {
            items,
            item_rects,
            menu_rect: [menu_left, menu_right, anchor_ndc[1], menu_bottom],
        }
    }
}
```

- [ ] **Step 4: 单测**

```rust
#[test]
fn sidebar_settings_menu_open_close() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    s.set_visibility(Visibility::Pinned);
    let input = SidebarInput {
        tabs: &[], active_index: None,
        screen_w: 1200.0, screen_h: 800.0,
        traffic_light_inset: (0.0, 0.0),
    };
    s.update_layout(&input, &cfg);
    s.open_settings_menu(crate::view_mode::ViewMode::Sidebar, 1200.0, 800.0);
    assert!(s.open_menu().is_some());
    // 模拟点 Sidebar 模式项中心
    let menu = s.open_menu().unwrap().clone();
    let r = menu.item_rects[0];
    let cx = (r[0] + r[1]) * 0.5;
    let cy = (r[2] + r[3]) * 0.5;
    let action = s.dispatch_menu_click(cx, cy);
    assert!(matches!(action, Some(SidebarAction::SetViewMode(crate::view_mode::ViewMode::Sidebar))));
    assert!(s.open_menu().is_none());
}
```

- [ ] **Step 5: 跑测试**

Run: `cargo test -p edit-plus-ui sidebar`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add crates/ui/src/sidebar.rs crates/ui/src/popup_menu.rs
git commit -m "ui: build sidebar settings popup menu and dispatcher"
```

### Task 8.2：app 接入设置菜单

**Files:**
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/settings_io.rs`

- [ ] **Step 1: SidebarAction::OpenSettingsMenu 在 apply_sidebar_action 中弹菜单**

```rust
SA::OpenSettingsMenu => {
    workspace.sidebar_state.open_settings_menu(
        Settings::get_static().view_mode,
        screen_w, screen_h,
    );
}
```

`screen_w/screen_h` 从调用上下文传入（apply_sidebar_action 接收两参数）。

- [ ] **Step 2: 当 sidebar.open_menu 存在时，全局 click 走 dispatch_menu_click**

events.rs 在 mouse_down 顶部：

```rust
if matches!(Settings::get_static().view_mode, ui::view_mode::ViewMode::Sidebar) {
    if workspace.sidebar_state.open_menu().is_some() {
        let ndc_x = px / screen_w * 2.0 - 1.0;
        let ndc_y = 1.0 - py / screen_h * 2.0;
        if let Some(action) = workspace.sidebar_state.dispatch_menu_click(ndc_x, ndc_y) {
            apply_sidebar_action(workspace, action, &mut actions, screen_w, screen_h);
        } else {
            // 点空白处也关菜单
            workspace.sidebar_state.set_open_menu(None);
        }
        return;
    }
}
```

`SidebarState::set_open_menu` 已有；如未暴露，加 setter。

- [ ] **Step 3: SidebarAction::SetViewMode 写盘 + 切换**

```rust
SA::SetViewMode(new_mode) => {
    {
        let mut s = Settings::get_mut();
        s.view_mode = new_mode;
    }
    crate::settings_io::save(&crate::settings_io::PersistedSettings { view_mode: new_mode });
    app.apply_view_mode(new_mode); // 触发 NSWindow titlebar 调整
}
```

> `apply_sidebar_action` 之前签名只取 `workspace, action, actions`；为了能拿到 `app.apply_view_mode`，把模式切换信号通过 `actions.push(AppAction::SetViewMode(new_mode));` 让主事件循环处理。最简：把 `Workspace` 之外的副作用交给 `AppAction`。

新增 `enum AppAction::SetViewMode(ui::view_mode::ViewMode)`，在 actions.rs 中加；并在 app.rs 主循环 dispatcher 中处理：

```rust
AppAction::SetViewMode(mode) => {
    {
        let mut s = ui::Settings::get_mut();
        s.view_mode = mode;
    }
    crate::settings_io::save(&crate::settings_io::PersistedSettings { view_mode: mode });
    self.apply_view_mode(mode);
    self.window.request_redraw();
}
```

- [ ] **Step 4: SidebarAction::OpenSettingsFile**

`apply_sidebar_action`:

```rust
SA::OpenSettingsFile => {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let path = std::path::PathBuf::from(home).join(".edit+").join("settings.yaml");
    if !path.exists() {
        // 写一份默认
        crate::settings_io::save(&crate::settings_io::PersistedSettings::default());
    }
    actions.push(AppAction::OpenFile(path));
}
```

`AppAction::OpenFile` 应已存在（Cmd+O 路径）；如不存在，复用 `AppAction::OpenPath`/相似变体。

- [ ] **Step 5: 启动手测**

Run: `cargo run -p edit-plus-app -- assets/samples/medium_ascii_5mb.txt`
预期：
- 点 ⚙ 设置 → 弹菜单
- 点 「Tabs 模式」→ NSWindow titlebar 还原 + Tab 栏出现 + 写入 settings.yaml
- 点 「Sidebar 模式」→ 反向恢复
- 点「打开 settings.yaml」→ 新 tab 打开 yaml 文件

- [ ] **Step 6: 提交**

```bash
git add crates/app/src/events.rs crates/app/src/app.rs crates/app/src/actions.rs crates/app/src/settings_io.rs
git commit -m "app: wire sidebar settings menu (mode switch + open settings.yaml)"
```

---

# 阶段 9：边界打磨 + 手动验证

**Goal**: 落实 spec §5.2 边界场景；扩文件列表内部滚动；手动验证文档落地。

### Task 9.1：极窄窗口禁止 Pinned

**Files:**
- Modify: `crates/ui/src/sidebar.rs`

- [ ] **Step 1: 改 update_layout，窄窗口强制 Hidden**

```rust
pub fn update_layout(&mut self, input: &SidebarInput<'_>, cfg: &SidebarConfig) {
    if matches!(self.visibility, Visibility::Pinned) && input.screen_w < cfg.width + 100.0 {
        self.visibility = Visibility::Hidden;
    }
    // ... 既有逻辑
}
```

- [ ] **Step 2: 单测**

```rust
#[test]
fn sidebar_extreme_narrow_window_disables_pin() {
    let cfg = SidebarConfig { pinned: true, width: 220.0 };
    let mut s = SidebarState::new(&cfg);
    let input = SidebarInput {
        tabs: &[], active_index: None,
        screen_w: 250.0, screen_h: 600.0, // 250 < 220+100
        traffic_light_inset: (0.0, 0.0),
    };
    s.update_layout(&input, &cfg);
    assert_eq!(s.visibility(), Visibility::Hidden);
}
```

- [ ] **Step 3: 跑测试**

Run: `cargo test -p edit-plus-ui sidebar`
Expected: PASS。

- [ ] **Step 4: 提交**

```bash
git add crates/ui/src/sidebar.rs
git commit -m "ui: force sidebar Hidden in extreme narrow window"
```

### Task 9.2：文件列表内部纵向滚动

**Files:**
- Modify: `crates/ui/src/sidebar.rs`

- [ ] **Step 1: 在 SidebarState 加 list_scroll_offset 与 on_scroll**

```rust
pub struct SidebarState {
    // ...
    list_scroll_px: f32,
}

impl SidebarState {
    pub fn on_scroll(&mut self, dy: f32, total_items: usize, list_height_px: f32) {
        let dpi = Settings::get().dpi_scale;
        let row_h = ROW_H * dpi;
        let total_h = total_items as f32 * row_h;
        let max = (total_h - list_height_px).max(0.0);
        self.list_scroll_px = (self.list_scroll_px + dy).clamp(0.0, max);
    }
}
```

`update_layout` 用 `self.list_scroll_px` 偏移文件项 y 起点。

- [ ] **Step 2: 单测**

```rust
#[test]
fn sidebar_list_scroll_clamps_to_content() {
    let mut s = SidebarState::default();
    s.set_visibility(Visibility::Pinned);
    s.on_scroll(9999.0, 100, 200.0);
    let dpi = 1.0;
    let max = (100.0 * 24.0 * dpi - 200.0).max(0.0);
    assert!((s.list_scroll_px - max).abs() < 1.0);
    s.on_scroll(-9999.0, 100, 200.0);
    assert_eq!(s.list_scroll_px, 0.0);
}
```

- [ ] **Step 3: app 转发滚轮事件**

`crates/app/src/events.rs` 滚轮 handler 加 sidebar 分支：

```rust
if matches!(Settings::get_static().view_mode, ui::view_mode::ViewMode::Sidebar)
    && px <= workspace.sidebar_cfg.width
    && workspace.sidebar_state.is_visible()
{
    let list_h = screen_h - 28.0 * Settings::get_static().dpi_scale - 28.0 * Settings::get_static().dpi_scale - 28.0 * Settings::get_static().dpi_scale;
    workspace.sidebar_state.on_scroll(-scroll_dy, workspace.doc_views.len(), list_h);
    return;
}
```

- [ ] **Step 4: 跑测 + 启动手测（100 tab）**

Run: `cargo test -p edit-plus-ui sidebar`
Run: 启动并 Cmd+T 连按 100 次创建 tab；侧边栏列表能纵向滚动。

- [ ] **Step 5: 提交**

```bash
git add crates/ui/src/sidebar.rs crates/app/src/events.rs
git commit -m "ui: scroll long file lists inside sidebar"
```

### Task 9.3：空 tab 占位文案

**Files:**
- Modify: `crates/ui/src/sidebar.rs`

- [ ] **Step 1: 在 text_positions 中检测 items 为空时加占位**

```rust
if layout.items.is_empty() {
    let cy_ndc = (layout.list_clip[2] + layout.list_clip[3]) * 0.5;
    let (_, py) = ndc_to_px(layout.list_clip[0], cy_ndc);
    let (px, _) = ndc_to_px(layout.list_clip[0], 0.0);
    out.push(SidebarText {
        text: "无打开文档".into(),
        x_px: px + 8.0 * Settings::get().dpi_scale,
        y_px: py,
        color: theme.sidebar_item_fg,
    });
}
```

- [ ] **Step 2: 单测**

```rust
#[test]
fn sidebar_empty_tabs_shows_placeholder() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    s.set_visibility(Visibility::Pinned);
    let input = SidebarInput {
        tabs: &[], active_index: None,
        screen_w: 1200.0, screen_h: 800.0,
        traffic_light_inset: (0.0, 0.0),
    };
    s.update_layout(&input, &cfg);
    let theme = crate::theme::Theme::dark();
    let texts = s.text_positions(1200.0, 800.0, &theme, 14.0);
    assert!(texts.iter().any(|t| t.text == "无打开文档"));
}
```

- [ ] **Step 3: 跑测试**

Run: `cargo test -p edit-plus-ui sidebar`
Expected: PASS。

- [ ] **Step 4: 提交**

```bash
git add crates/ui/src/sidebar.rs
git commit -m "ui: render placeholder when sidebar tab list empty"
```

### Task 9.4：模式切换时丢弃临时状态

**Files:**
- Modify: `crates/app/src/app.rs`

- [ ] **Step 1: apply_view_mode 中清理 sidebar 状态**

```rust
fn apply_view_mode(&mut self, view_mode: ui::view_mode::ViewMode) {
    match view_mode {
        ui::view_mode::ViewMode::Sidebar => crate::sys::macos_titlebar::enable_full_size_content(&self.window),
        ui::view_mode::ViewMode::Tabs    => crate::sys::macos_titlebar::disable_full_size_content(&self.window),
    }
    // 清理 sidebar overlay / drag / menu，避免残留
    self.workspace.sidebar_state.set_open_menu(None);
    if matches!(view_mode, ui::view_mode::ViewMode::Tabs) {
        self.workspace.sidebar_state.set_visibility(ui::sidebar::Visibility::Hidden);
        self.workspace.dragging_sidebar = false;
    }
    self.window.request_redraw();
}
```

- [ ] **Step 2: 集成测试 view_mode_switch_no_panic**

在 `crates/app/tests/` 加 `view_mode_switch.rs`：

```rust
//! Smoke test: switching view mode 10 times does not panic.

#[test]
fn view_mode_switch_no_panic() {
    use ui::view_mode::ViewMode;
    use edit_plus_app::settings_io::{load, save, PersistedSettings};

    let original = load();
    for i in 0..10 {
        let mode = if i % 2 == 0 { ViewMode::Sidebar } else { ViewMode::Tabs };
        save(&PersistedSettings { view_mode: mode });
        let read = load();
        assert_eq!(read.view_mode, mode);
    }
    save(&original); // 还原
}
```

> 需要把 `crates/app/src/settings_io.rs` 的 `pub(crate)` 改成 `pub`，并在 `crates/app/src/lib.rs` 里 `pub mod settings_io;`。

- [ ] **Step 3: 跑测试**

Run: `cargo test -p edit-plus-app view_mode_switch`
Expected: PASS。

- [ ] **Step 4: 提交**

```bash
git add crates/app/src/app.rs crates/app/tests/view_mode_switch.rs crates/app/src/lib.rs crates/app/src/settings_io.rs
git commit -m "app: reset sidebar transient state on view-mode switch + smoke test"
```

### Task 9.5：手动测试协议

**Files:**
- Modify: `docs/manual_test_protocol.md`

- [ ] **Step 1: 追加 §10 阶段：sidebar 双模式**

读取 `docs/manual_test_protocol.md`（如不存在则创建）。在文末追加：

```markdown
## §10 Sidebar 双模式（2026-06-11）

### M10.1 默认 sidebar 启动
命令：`cargo run --release -p edit-plus-app -- assets/samples/medium_ascii_5mb.txt`
预期：
- ✅ 启动后红绿灯位于 sidebar header（macOS）
- ✅ 默认未钉住，不显示 sidebar；编辑区贴近左边
- ❌ 不允许任何 panic 或 GPU 警告

### M10.2 hover 弹出
预期：
- ✅ 鼠标进入窗口左 4px 热区，停 ~150ms 后 sidebar overlay 出现
- ✅ 鼠标离开 sidebar 区 ~300ms 后消失
- ✅ 按 Esc 立即收起 overlay

### M10.3 钉住 / 取消钉住
预期：
- ✅ Cmd+B 切钉住，编辑区水平让位 sidebar 宽度
- ✅ 再按 Cmd+B 取消钉住，编辑区还原
- ✅ 钉住状态下重启 app 仍钉住

### M10.4 边缘拖拽改宽
预期：
- ✅ 钉住状态下，鼠标到右边缘 4px 内显示 ↔ 光标
- ✅ 拖拽改宽，最小 160 * dpi、最大 400 * dpi
- ✅ 松手后重启 app 宽度保持

### M10.5 设置菜单切模式
预期：
- ✅ 点 ⚙ 设置弹菜单：「Sidebar 模式 ✓ / Tabs 模式 / 打开 settings.yaml」
- ✅ 选 Tabs 模式：红绿灯回原生位、Tab 栏出现、settings.yaml 写入新值
- ✅ 选 Sidebar 模式：反向恢复

### M10.6 100+ tab 列表
预期：
- ✅ Cmd+T 连按 100 次后 sidebar 文件列表内部纵向滚动顺滑
- ✅ 点击列表项切换正确

### M10.7 极窄窗口
预期：
- ✅ 把窗口缩到 < (sidebar_width + 100) px，sidebar 自动收起 / 禁止钉住
- ✅ 还原宽度后可重新钉住

### M10.8 全屏 / Stage Manager
预期：
- ✅ 全屏切换不破坏布局
- ✅ Stage Manager 切换不破坏 traffic_light_inset
```

- [ ] **Step 2: 提交**

```bash
git add docs/manual_test_protocol.md
git commit -m "docs: add manual test protocol §10 for sidebar dual mode"
```

### Task 9.6：clippy / fmt 总扫

**Files:**
- 全仓

- [ ] **Step 1: clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: 0 warning。

- [ ] **Step 2: fmt**

Run: `cargo fmt --all --check`
Expected: 0 diff。如有则 `cargo fmt --all` 修后提交。

- [ ] **Step 3: cargo test 全套**

Run: `cargo test --workspace`
Expected: 全 PASS。

- [ ] **Step 4: cargo build --release 验证**

Run: `cargo build --release -p edit-plus-app`
Expected: 0 错误，无新警告。

- [ ] **Step 5: 提交（如有 fmt 改动）**

```bash
git add -u
git commit -m "chore: cargo fmt after sidebar implementation"
```

---

# 自检与最后落地

到此 9 个阶段完成。最后做一次仪表盘对照：

- spec §1 目标 → 阶段 1-9 全部覆盖
- spec §2 用户体验 → 阶段 4 (默认/状态机) + 阶段 5 (titlebar) + 阶段 6 (hover/Cmd+B/Esc) + 阶段 8 (模式切换) + 阶段 9 (边界)
- spec §3 架构 → 阶段 1 (popup_menu) + 阶段 2 (TabBarState) + 阶段 4-7 (SidebarState) + 阶段 5 (titlebar)
- spec §4 持久化 → 阶段 3 (字段 + 加载) + 阶段 7 (drag_end 写盘) + 阶段 8 (settings.yaml 写盘)
- spec §5 错误处理与边界 → 阶段 9
- spec §6 测试 → 各阶段单测 + 阶段 9 集成测试
- spec §7 阶段切分 → 完全对齐
- spec §8 风险与决策 → 阶段 5 / 阶段 7 缓解措施已落地

完成后建议把 `view_mode` 默认从 `Tabs` 改为 `Sidebar`（已在 Task 4.6 执行）。

---

## 注意事项

1. **每阶段可独立编译可独立运行**。如果某阶段中途阻塞，先 commit 部分进度，回头再处理。
2. **objc2 API 路径可能与示例略有出入**：阶段 5 实现时以本机 `objc2-app-kit` 0.3.x 文档为准，调整 `NSWindowButton::CloseButton` 等具体调用。
3. **theme 颜色值** 是初版近似；阶段 9 之后可让设计师/用户调。
4. **font 字号**：sidebar 文字暂用 `Settings::get().font_size * 0.85` 或固定 `13.0 * dpi_scale`；具体调用处自行选择。本计划未硬编码，避免误导。
5. **AppAction 列表**：本计划假设 `AppAction::SwitchTab(usize)` / `NewEmptyTab` / `OpenFile(PathBuf)` / `SetViewMode(ViewMode)` 已存在或顺手添加；新增前先在 `crates/app/src/actions.rs` 检查。
