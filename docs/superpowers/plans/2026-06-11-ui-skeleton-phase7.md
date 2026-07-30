# UI 骨架 Phase 7：sidebar widget 化

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `crates/ui/src/sidebar.rs`（464 行）改造为 `SidebarWidget`：内部全 px、删 `to_ndc` 闭包与 `fill_quad` helper、`vertices` + `text_positions` 合并为 `paint(&mut PaintCtx)`。事件路径走 `ui_shell.dispatch` 返回 `SidebarAction`；老 `SidebarState::vertices / text_positions` 删除。Phase 7 不接 popup（settings 菜单仍走老 popup_menu，Phase 8 处理）。

**Architecture:**
- `SidebarLayout` 矩形从 NDC `[f32; 4]` 改为 `Rect`(px)。
- `SidebarWidget` 持 `SidebarState + SidebarConfig`；`set_input(...)` 由 app 注入 tabs / active_index / traffic_light_inset。
- `paint`：bg + header + 菜单按钮 + 新建按钮 + items + settings 按钮 + 边缘 resize bar。文字通过 `DrawList::text` 输出。
- `on_event`：MouseDown 走 hit_test_px → SidebarAction；MouseDown 在 edge resize bar 上 → `StartResize`；MouseMove + dragging → `ResizeTo(width)`；MouseUp → `EndResize`。

**Tech Stack:** Rust 2024 · 复用 `paint_backend` 的 Text 路径 · 现有 `SidebarConfig / Visibility` 不动。

**Spec：** `docs/superpowers/specs/2026-06-11-ui-skeleton-design.md` §6（sidebar 行）、§7（阶段 7）

---

## 文件结构

| 文件 | 改动类型 | 备注 |
|---|---|---|
| `crates/ui/src/sidebar.rs` | Modify | layout 改 px；删 to_ndc/fill_quad；`vertices/text_positions` 删；新增 `paint(&PaintCtx)`、`hit_test_px`、resize 状态 |
| `crates/ui/src/widgets/sidebar.rs` | Create | `SidebarWidget` |
| `crates/ui/src/widgets/mod.rs` | Modify | `pub mod sidebar;` |
| `crates/ui/src/lib.rs` | Modify | re-export |
| `crates/app/src/ui_shell.rs` | Modify | 真 widget 注册 + `set_sidebar_input` |
| `crates/app/src/app_renderer.rs` | Modify | 删除 sidebar 渲染分支 |
| `crates/app/src/events.rs / app.rs` | Modify | sidebar 鼠标事件改走 widget |

---

## Task 1：sidebar.rs layout px 化 + 删裸 fn

**Files:**
- Modify: `crates/ui/src/sidebar.rs`

- [ ] **Step 1.1：layout 改 px**

读 `crates/ui/src/sidebar.rs` 完整内容（464 行）。

把 `SidebarLayout` 矩形字段全部改 `Rect`(px)：

```rust
use crate::core::Rect;

#[derive(Debug, Clone, Default)]
pub struct SidebarLayout {
    pub bg_rect: Rect,
    pub header_rect: Rect,
    pub menu_btn_rect: Rect,
    pub new_btn_rect: Rect,
    pub items: Vec<SidebarLayoutItem>,
    pub list_clip: Rect,
    pub settings_btn_rect: Rect,
    pub edge_resize_rect: Rect,
}

#[derive(Debug, Clone)]
pub struct SidebarLayoutItem {
    pub tab_index: usize,
    pub rect: Rect,           // px
    pub title: String,
    pub indicator: TabIndicator,
}
```

`update_layout` 内部完整改写：删掉 `to_ndc` 闭包，所有矩形赋值都改成 `Rect::new(x_px, y_px, w_px, h_px)`。

把 `vertices` 与 `text_positions` 方法**删除**。新增 `paint`：

```rust
use crate::core::PaintCtx;

impl SidebarState {
    pub fn paint(&self, ctx: &mut PaintCtx, active_index: Option<usize>) {
        let Some(layout) = &self.layout else { return; };

        // 1) 背景
        ctx.list.fill(layout.bg_rect, ctx.theme.sidebar_bg);
        // 2) 头部
        ctx.list.fill(layout.header_rect, ctx.theme.sidebar_header_bg);
        // 3) 新建按钮
        ctx.list.fill(layout.new_btn_rect, ctx.theme.sidebar_button_bg);
        // 4) 设置按钮
        ctx.list.fill(layout.settings_btn_rect, ctx.theme.sidebar_header_bg);

        // 5) 列表项 + 标题文本
        let font_size = 13.0 * ctx.dpi;
        let pad_left = 8.0 * ctx.dpi;
        for item in &layout.items {
            let bg = if Some(item.tab_index) == active_index {
                ctx.theme.sidebar_item_active_bg
            } else {
                ctx.theme.sidebar_item_bg
            };
            ctx.list.fill(item.rect, bg);
            // 文字（与 Phase 3 status_bar 同款 baseline）
            let baseline = item.rect.y + item.rect.h * 0.65;
            ctx.list.text(
                item.rect.x + pad_left,
                baseline,
                font_size,
                ctx.theme.sidebar_item_fg,
                &item.title,
            );
        }
    }
}
```

- [ ] **Step 1.2：hit_test 改 px**

读 `SidebarState::hit_test_at(px, py, screen_w, screen_h)` —— 它把 px 转 NDC 再比较；现在直接用 px 比 Rect.contains：

```rust
impl SidebarState {
    pub fn hit_test_px(&self, px: f32, py: f32) -> Option<SidebarAction> {
        let layout = self.layout.as_ref()?;

        if layout.menu_btn_rect.contains(px, py) {
            return Some(SidebarAction::TogglePin);
        }
        if layout.new_btn_rect.contains(px, py) {
            return Some(SidebarAction::NewDocument);
        }
        if layout.settings_btn_rect.contains(px, py) {
            return Some(SidebarAction::OpenSettingsMenu);
        }
        if layout.edge_resize_rect.contains(px, py) {
            return Some(SidebarAction::StartResize);
        }
        for item in &layout.items {
            if item.rect.contains(px, py) {
                return Some(SidebarAction::SwitchTab(item.tab_index));
            }
        }
        None
    }
}
```

`SidebarAction` 增加：

```rust
pub enum SidebarAction {
    SwitchTab(usize),
    NewDocument,
    OpenSettingsMenu,
    ToggleViewMode,
    TogglePin,
    SetWidth(f32),
    Context { action: ContextMenuAction, tab_index: usize },
    /// 新增：开始拖拽边缘 resize
    StartResize,
    /// 新增：拖拽中
    ResizeTo(f32),
    /// 新增：松手
    EndResize,
}
```

**删除**老 `hit_test_at`、`fill_quad`、`vertices`、`text_positions`。

`SidebarState` 内部增加：

```rust
pub struct SidebarState {
    visibility: Visibility,
    layout: Option<SidebarLayout>,
    open_menu: Option<PopupMenu>,
    /// 新增：是否在拖拽 resize
    resizing: bool,
}
```

并在 `new` 初始化为 false。

- [ ] **Step 1.3：测试更新**

读 `sidebar.rs` 的 `#[cfg(test)] mod tests`。把现有用 NDC 验证的测试改成 px：

```rust
#[test]
fn sidebar_click_file_emits_switch_tab() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut s = SidebarState::new(&cfg);
    let tabs = vec![
        TabInfo { title: "a.rs".into(), file_path: None, is_dirty: false, language: "rust".into() },
    ];
    let input = SidebarInput {
        tabs: &tabs, active_index: Some(0),
        screen_w: 1200.0, screen_h: 800.0,
        traffic_light_inset: (0.0, 0.0),
    };
    s.update_layout(&input, &cfg);
    let layout = s.current_layout().unwrap();
    let item = &layout.items[0];
    let px = item.rect.x + item.rect.w * 0.5;
    let py = item.rect.y + item.rect.h * 0.5;
    let action = s.hit_test_px(px, py);
    assert!(matches!(action, Some(SidebarAction::SwitchTab(0))));
}
```

把所有用 `(cx_ndc + 1.0) * 0.5 * sw` 反算 px 的测试简化成直接拿 rect 中心。

- [ ] **Step 1.4：跑测试**

```bash
cargo test -p edit-plus-ui sidebar
```

预期：通过。

- [ ] **Step 1.5：提交**

```bash
git add crates/ui/src/sidebar.rs
git commit -m "refactor(ui-sidebar): layout 改 Rect(px)；删 to_ndc/fill_quad/vertices；加 paint + hit_test_px"
```

---

## Task 2：SidebarWidget

**Files:**
- Create: `crates/ui/src/widgets/sidebar.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

- [ ] **Step 2.1：实现**

```rust
//! SidebarWidget — 持 SidebarState + 对外暴露 set_input / set_visibility / set_active。

use std::any::Any;

use crate::core::{Widget, Rect, LayoutCtx, PaintCtx, EventCtx, Event, MouseButton};
use crate::sidebar::{SidebarState, SidebarConfig, SidebarInput, SidebarAction, Visibility};
use crate::tab_bar::TabInfo;

pub struct SidebarWidget {
    state: SidebarState,
    cfg: SidebarConfig,
    rect: Rect,
    /// 入参缓存：tabs / active_index / traffic_light
    tabs: Vec<TabInfo>,
    active_index: Option<usize>,
    traffic_light_inset: (f32, f32),
    screen_w: f32,
    screen_h: f32,
    /// 拖拽起点（鼠标按下时记录，drag 时反算 width）
    drag_anchor_x: Option<f32>,
}

impl SidebarWidget {
    pub fn new(dpi: f32) -> Self {
        let cfg = SidebarConfig::new_default(dpi);
        let state = SidebarState::new(&cfg);
        Self {
            state, cfg,
            rect: Rect::ZERO,
            tabs: Vec::new(),
            active_index: None,
            traffic_light_inset: (0.0, 0.0),
            screen_w: 0.0, screen_h: 0.0,
            drag_anchor_x: None,
        }
    }

    pub fn set_visibility(&mut self, v: Visibility) { self.state.set_visibility(v); }
    pub fn visibility(&self) -> Visibility { self.state.visibility() }
    pub fn cfg(&self) -> &SidebarConfig { &self.cfg }
    pub fn cfg_mut(&mut self) -> &mut SidebarConfig { &mut self.cfg }
    pub fn current_width(&self) -> f32 { self.state.current_width(&self.cfg) }
    pub fn editor_left_offset(&self) -> f32 { self.state.editor_left_offset(&self.cfg) }

    pub fn set_input(
        &mut self,
        tabs: Vec<TabInfo>,
        active_index: Option<usize>,
        traffic_light_inset: (f32, f32),
        screen_w: f32, screen_h: f32,
    ) {
        self.tabs = tabs;
        self.active_index = active_index;
        self.traffic_light_inset = traffic_light_inset;
        self.screen_w = screen_w;
        self.screen_h = screen_h;
    }
}

impl Widget for SidebarWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
        // 重算 layout（用最新 input + 当前宽度）
        let input = SidebarInput {
            tabs: &self.tabs,
            active_index: self.active_index,
            screen_w: self.screen_w,
            screen_h: self.screen_h,
            traffic_light_inset: self.traffic_light_inset,
        };
        self.state.update_layout(&input, &self.cfg);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if !self.state.is_visible() { return; }
        self.state.paint(ctx, self.active_index);
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.state.is_visible() && self.rect.contains(px, py)
    }

    fn on_event(&mut self, ev: &Event, _ctx: &mut EventCtx) -> Option<Box<dyn Any>> {
        match ev {
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                let action = self.state.hit_test_px(*px, *py)?;
                if matches!(action, SidebarAction::StartResize) {
                    self.drag_anchor_x = Some(*px);
                }
                Some(Box::new(action))
            }
            Event::MouseMove { px, .. } => {
                if let Some(anchor) = self.drag_anchor_x {
                    let new_w = self.cfg.width + (*px - anchor);
                    let mut clamped = SidebarConfig { pinned: self.cfg.pinned, width: new_w };
                    let dpi_for_clamp = 1.0; // 入参 dpi 不必每次重算 clamp 边界
                    let _ = dpi_for_clamp;   // 简化：clamp 在 cfg 自己里
                    clamped.clamp_width(1.0);
                    self.cfg.width = clamped.width;
                    self.drag_anchor_x = Some(*px);
                    Some(Box::new(SidebarAction::ResizeTo(self.cfg.width)))
                } else { None }
            }
            Event::MouseUp { button: MouseButton::Left, .. } => {
                if self.drag_anchor_x.take().is_some() {
                    Some(Box::new(SidebarAction::EndResize))
                } else { None }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{NoopMeasure, DrawList};
    use crate::Theme;

    fn layout_ctx<'a>(theme: &'a Theme, m: &'a mut dyn crate::core::TextMeasure) -> LayoutCtx<'a> {
        LayoutCtx { measure: m, theme, dpi: 1.0 }
    }

    #[test]
    fn invisible_paint_emits_nothing() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = SidebarWidget::new(1.0);
        w.set_visibility(Visibility::Hidden);
        w.set_input(Vec::new(), None, (0.0, 0.0), 1200.0, 800.0);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut layout);

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        assert!(list.is_empty());
    }

    #[test]
    fn pinned_paint_emits_bg_header_buttons() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = SidebarWidget::new(1.0);
        w.set_visibility(Visibility::Pinned);
        w.set_input(Vec::new(), None, (0.0, 0.0), 1200.0, 800.0);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut layout);

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        // bg + header + new_btn + settings_btn = 至少 4
        assert!(list.len() >= 4);
    }

    #[test]
    fn click_settings_btn_emits_open_settings_menu() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = SidebarWidget::new(1.0);
        w.set_visibility(Visibility::Pinned);
        w.set_input(Vec::new(), None, (0.0, 0.0), 1200.0, 800.0);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut layout);

        let layout_ref = w.state.current_layout().unwrap().clone();
        let cx = layout_ref.settings_btn_rect.x + layout_ref.settings_btn_rect.w * 0.5;
        let cy = layout_ref.settings_btn_rect.y + layout_ref.settings_btn_rect.h * 0.5;

        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        let action = w.on_event(
            &Event::MouseDown { px: cx, py: cy, button: MouseButton::Left },
            &mut ctx,
        ).unwrap();
        let typed = action.downcast::<SidebarAction>().unwrap();
        assert!(matches!(*typed, SidebarAction::OpenSettingsMenu));
    }

    #[test]
    fn drag_resize_updates_cfg_width() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = SidebarWidget::new(1.0);
        w.set_visibility(Visibility::Pinned);
        w.set_input(Vec::new(), None, (0.0, 0.0), 1200.0, 800.0);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut layout);

        // 假设 edge_resize_rect 在 (216, 28, 4, 772) 附近，按下 218
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        w.on_event(&Event::MouseDown { px: 218.0, py: 400.0, button: MouseButton::Left }, &mut ctx);
        let action = w.on_event(&Event::MouseMove { px: 268.0, py: 400.0 }, &mut ctx).unwrap();
        let typed = action.downcast::<SidebarAction>().unwrap();
        match *typed {
            SidebarAction::ResizeTo(width) => assert!((width - 270.0).abs() < 1.0),
            _ => panic!("expected ResizeTo"),
        }
    }
}
```

修改 `widgets/mod.rs` 与 `lib.rs` 加 re-export。

- [ ] **Step 2.2：跑测试**

```bash
cargo test -p edit-plus-ui widgets::sidebar
```

预期：4 个测试通过。

- [ ] **Step 2.3：提交**

```bash
git add crates/ui/src/widgets/sidebar.rs crates/ui/src/widgets/mod.rs crates/ui/src/lib.rs
git commit -m "feat(ui-widgets): sidebar — Widget + 拖拽 resize 支持"
```

---

## Task 3：UiShell 接 sidebar widget

**Files:**
- Modify: `crates/app/src/ui_shell.rs`

- [ ] **Step 3.1：替换占位**

```rust
use ui::widgets::sidebar::SidebarWidget;
use ui::sidebar::{Visibility, SidebarAction};

// 在 new() 里：
let idx_sidebar = {
    let idx = dock.children.len();
    let t_const = 0.0_f32;
    dock.push(DockChild::left(SidebarWidget::new(1.0), move |_, _| t_const));
    idx
};
```

新增方法：

```rust
impl UiShell {
    pub fn set_sidebar_input(
        &mut self,
        tabs: Vec<ui::tab_bar::TabInfo>,
        active_index: Option<usize>,
        traffic_light_inset: (f32, f32),
        screen_w: f32, screen_h: f32,
        visibility: Visibility,
    ) {
        let any = self.dock.children[self.idx_sidebar].widget.as_any_mut();
        if let Some(w) = any.downcast_mut::<SidebarWidget>() {
            w.set_visibility(visibility);
            w.set_input(tabs, active_index, traffic_light_inset, screen_w, screen_h);
        }
    }

    pub fn sidebar_current_width(&self) -> f32 {
        let any = (&self.dock.children[self.idx_sidebar].widget).as_any();
        any.downcast_ref::<SidebarWidget>().map(|w| w.current_width()).unwrap_or(0.0)
    }
}
```

> ⚠️ thickness 入参从 `build_shell_inputs()` 来；现在 sidebar widget 自己也持 cfg.width。两边可能不同步。**统一**：让 `build_shell_inputs::sidebar_thickness` 直接读 `ui_shell.sidebar_current_width()`。即：

`app.rs::build_shell_inputs`：

```rust
sidebar_thickness: self.ui_shell.sidebar_current_width(),
```

但这要求 sidebar widget 已经接收过 set_visibility，否则始终返回 0。流程顺序：

1. `set_sidebar_input(visibility=...)` → widget 拿到 visibility
2. `build_shell_inputs()` → 读 widget.current_width()
3. `update_frame(inputs)` → dock 用 thickness 算 layout

→ **必须**先 set_sidebar_input，再 build_shell_inputs，再 update_frame。在 app_renderer 里调整顺序。

- [ ] **Step 3.2：测试**

```bash
cargo test --workspace
```

- [ ] **Step 3.3：提交**

```bash
git add crates/app/src/ui_shell.rs
git commit -m "feat(app): ui_shell — sidebar 真 widget + width 由 widget 权威"
```

---

## Task 4：app_renderer / events 接入

**Files:**
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/app.rs`

- [ ] **Step 4.1：app_renderer set_sidebar_input + 删除老分支**

`app_renderer::render`：

在 build_shell_inputs 之**前**追加：

```rust
{
    use ui::sidebar::Visibility;
    use ui::view_mode::ViewMode;
    let view_mode = Settings::get_static().view_mode;
    let visibility = match view_mode {
        ViewMode::Sidebar => Visibility::Pinned,
        ViewMode::Tabs    => Visibility::Hidden,
    };
    self.ui_shell.set_sidebar_input(
        tab_infos.clone(),       // 重用上面构造的 tab_infos
        Some(self.workspace.active_index),
        (0.0, 0.0),              // traffic_light_inset 暂为 0；阶段 8 接红绿灯偏移
        screen_w, screen_h,
        visibility,
    );
}
```

**删除**老 `match view_mode` 块里 `ui::view_mode::ViewMode::Sidebar` 分支的 `update_layout / vertices` 调用。**保留**最外层 match（用于 tab_bar 的 set_tabs_input），但 sidebar 分支只剩 noop（visibility 已通过 set_sidebar_input 处理）。

删除：

```rust
ui::view_mode::ViewMode::Sidebar => {
    let sidebar_input = ui::sidebar::SidebarInput { ... };
    vertices.extend(self.workspace.sidebar_state.vertices(...));
}
```

- [ ] **Step 4.2：events / app 鼠标事件改走 widget**

读 `crates/app/src/events.rs / mouse.rs / app.rs`，所有 `workspace.sidebar_state.hit_test_at(...)` 调用替换为通过 ui_shell.dispatch 路径，类似 Phase 5 / 6 的做法：

```rust
{
    use ui::sidebar::SidebarAction;
    let ev = ui::core::Event::MouseDown { px, py, button: ui::core::MouseButton::Left };
    if let Some(boxed) = app.ui_shell.dispatch(&ev, &app.current_theme,
        ui::settings::Settings::get().dpi_scale)
    {
        if let Ok(typed) = boxed.downcast::<SidebarAction>() {
            actions.push(translate_sidebar_action(*typed));
            return actions;
        }
    }
}
```

`events.rs::translate_sidebar_action`：

```rust
fn translate_sidebar_action(a: ui::sidebar::SidebarAction) -> AppAction {
    use ui::sidebar::SidebarAction as S;
    match a {
        S::SwitchTab(i)        => AppAction::SwitchTab(i),
        S::NewDocument         => AppAction::NewEmptyTab,
        S::OpenSettingsMenu    => AppAction::OpenSettingsMenu,
        S::ToggleViewMode      => AppAction::ToggleViewMode,
        S::TogglePin           => AppAction::TogglePin,
        S::SetWidth(w)         => AppAction::SetSidebarWidth(w),
        S::Context { action, tab_index } =>
            AppAction::ExecuteContextMenuAction(action, tab_index),
        S::StartResize         => AppAction::SidebarResizeStart,
        S::ResizeTo(w)         => AppAction::SetSidebarWidth(w),
        S::EndResize           => AppAction::SidebarResizeEnd,
    }
}
```

> ⚠️ 这些 AppAction 中可能有些不存在；按需在 `crates/app/src/actions.rs` 添加。`SetSidebarWidth(f32)` 调用方应同步把宽度写回 `workspace.sidebar_cfg`（Phase 8 后再考虑彻底删 workspace.sidebar_cfg；本阶段保留双写以减少爆炸面）。

- [ ] **Step 4.3：删 workspace.sidebar_state？**

不删。原因同 Phase 6 tab_bar：太多代码引用 `workspace.sidebar_state.update_layout / current_layout / current_width`。本阶段双轨，Phase 9 收尾删。

- [ ] **Step 4.4：build && run**

```bash
cargo build --workspace
cargo test --workspace
cargo run -p edit-plus-app -- README.md
```

切到 sidebar 模式（按对应快捷键）：
- 列表项点击切换文档
- 右上角 ☰ 切 pin
- 底部"+"新建
- 底部齿轮"打开设置菜单"（弹出 popup —— Phase 8 才接 widget；这里走老路径，验证不闪退即可）
- 拖动右边缘改宽度
- DPI 缩放正确

- [ ] **Step 4.5：分提交**

```bash
git add crates/app/src/app_renderer.rs
git commit -m "refactor(app): sidebar 走 ui_shell paint，删老 vertices 分支"

git add crates/app/src/events.rs crates/app/src/app.rs crates/app/src/actions.rs
git commit -m "refactor(app): sidebar 鼠标事件走 widget dispatch + resize 路径"
```

---

## Task 5：Phase 7 收尾

- [ ] **Step 5.1：grep**

```bash
grep -rn "sidebar_state.vertices\|sidebar_state.text_positions\|to_ndc.*sidebar\|fill_quad" crates/
```

预期：仅命中文档 / 注释。

- [ ] **Step 5.2：手测**

切换 sidebar/tab 模式来回，确认宽度持久（cfg.width 写回 workspace；resize 后保存 snapshot），DPI 切换正确。

- [ ] **Step 5.3：spec 追加**

```markdown
## Phase 7 完工记录

- 改造：sidebar.rs layout 改 px；vertices/text_positions 删；新增 paint + hit_test_px
- 接入：SidebarWidget；resize 拖拽走 widget
- 双轨：workspace.sidebar_state / sidebar_cfg 仍存在（Phase 9 删）
- 后续：Phase 8 接 popup_menu overlay
```

```bash
git add docs/superpowers/specs/2026-06-11-ui-skeleton-design.md
git commit -m "docs(spec): UI 骨架 Phase 7 完工记录"
```

---

## 边界情况清单

1. **sidebar 隐藏 ↔ 显示切换**：visibility=Hidden → paint 空 + thickness 0 → dock 跳过；切回 Pinned 时 widget 重新 layout，无残留状态。
2. **resize 拖到最小/最大**：clamp_width 内置 160~400 范围（与 dpi 相关）；拖出范围会卡住。
3. **HoverPeek 状态**（鼠标接近左侧弹出）：本阶段保留老 `Visibility::HoverPeek` 状态机，但 widget 还没接管这个触发器；hover 触发逻辑仍在 events.rs 老路径。Phase 9 收尾时考虑接管。
4. **traffic_light_inset**（macOS 红绿灯）：本阶段 `(0.0, 0.0)`；header layout 不会避让。Phase 9 接入实际值。
5. **settings 菜单按钮**：单击触发 `OpenSettingsMenu` → app 仍走老 popup。Phase 8 popup 接入后 widget 路径自动统一。
6. **CJK 文件名**：item paint 用 `DrawList::text(...)`，paint_backend 已支持。
7. **sidebar 宽度持久化**：`SetSidebarWidth(w)` action 调用方应写回 `workspace.sidebar_cfg.width`，并触发 snapshot。
