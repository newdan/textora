# UI 骨架 Phase 6：tab_bar 拆分 + widget 化

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 1656 行的 `crates/ui/src/tab_bar.rs` 拆成 `crates/ui/src/tab_bar/` 子目录下的多个文件，并把"绘制接口"切到 widget 路径。`app_renderer.rs::tab_text_vertices`（230 行）整段删除——文字渲染走 `paint_backend` 的 Text 路径。事件路径接入 `ui_shell.dispatch`。

**Architecture:**
- 拆分思路按 spec §6 表："tab_bar/{layout, state, widget, hit}"。`Layout` 矩形改 `Rect`(px) 形态；`vertices()` 删除，改 `paint(&mut PaintCtx)`。`TabBarAction` 保留（强类型，downcast）。
- 因为 `tab_bar.rs` 是 1656 行的"上帝模块"，CLAUDE.md 第 4 条要求"3 文件以上拆"——我们一次性拆成 6 文件，每文件 ≤ 400 行。
- 新增 `widgets/tab_bar.rs`（薄外壳），内部委托给 `ui::tab_bar::*` 子模块。
- 老 `tab_bar.rs` 文件改为 `tab_bar/mod.rs`；保留对外 re-export 名字（`TabInfo / TabBarAction / TabBarState / popup_menu_*` 等）以减小爆炸面。

**Tech Stack:** Rust 2024 · `ui::core::DrawList` · 现有 atlas+shaping。

**Spec：** `docs/superpowers/specs/2026-06-11-ui-skeleton-design.md` §6 表第 "tab_bar.rs(1656)" 行、§7（阶段 6）

---

## 文件结构（拆分后）

| 文件 | 行数预算 | 职责 |
|---|---|---|
| `crates/ui/src/tab_bar/mod.rs` | ~80 | re-export + `TabBarCtx / TabInfo / tab_bar_height` 等顶层共享 |
| `crates/ui/src/tab_bar/layout.rs` | ~400 | `TabBarLayout / TabEntry / NavButtonLayout / layout_tabs / max_tab_scroll / clamp_tab_scroll / set_preview_tab / compute_disambiguation` |
| `crates/ui/src/tab_bar/render.rs` | ~350 | 老 `tab_bar_vertices / tab_bar_text_positions / push_quad / push_rounded_rect`（保留供 popup_menu 复用） |
| `crates/ui/src/tab_bar/hit.rs` | ~150 | `TabHit / hit_test` |
| `crates/ui/src/tab_bar/state.rs` | ~200 | `TabBarInput / TabBarAction / TabBarState`；删 `vertices()`，改 `to_drawlist(...)` |
| `crates/ui/src/tab_bar/text.rs` | ~80 | `truncate_title_by_width / estimate_text_width_px / char_width`（被 layout 与 popup_menu 复用） |
| `crates/ui/src/widgets/tab_bar.rs` | ~150 | `TabBarWidget`：包装 `TabBarState`，实现 Widget trait |
| `crates/ui/src/popup_menu.rs` | 修改 | 把 `crate::tab_bar::{push_quad, push_rounded_rect, truncate_title_by_width, ...}` 引用改为新路径 `crate::tab_bar::{render, text}` |

> ⚠️ 拆分阶段**只搬不重写**：不修改任何函数实现，只把它们按职责分散到新文件并 `pub use` 回模块根。这一步保证拆分本身可逆。
> 真正的"NDC → Rect"改造留到 Task 4-5（widget 化时）。

---

## Task 1：tab_bar 文件拆分（不改实现）

**Files:**
- Create: `crates/ui/src/tab_bar/mod.rs`
- Create: `crates/ui/src/tab_bar/{layout,render,hit,state,text}.rs`
- Delete: `crates/ui/src/tab_bar.rs`（搬走后删）

- [ ] **Step 1.1：把 tab_bar.rs 备份并切成 6 文件**

```bash
mkdir -p crates/ui/src/tab_bar
mv crates/ui/src/tab_bar.rs crates/ui/src/tab_bar/_orig.rs.bak
```

按下面分配，把 `_orig.rs.bak` 中函数挨个搬到对应文件。每个新文件顶部加：

```rust
//! tab_bar/<file>.rs — 从 tab_bar.rs 拆出，无实现改动。

use super::*;       // 从 tab_bar/mod.rs 拿 re-export
```

**`tab_bar/text.rs`**（搬：`char_width / truncate_title_by_width / estimate_text_width_px`）

**`tab_bar/layout.rs`**（搬：`TabIndicator / TabEntry / TabBarLayout / NavButtonLayout / compute_disambiguation / max_tab_scroll / set_preview_tab / clamp_tab_scroll / layout_tabs`）

**`tab_bar/render.rs`**（搬：`tab_bar_vertices / tab_bar_text_positions / push_quad / push_rounded_rect`）

**`tab_bar/hit.rs`**（搬：`TabHit / hit_test / MouseButton`）

**`tab_bar/state.rs`**（搬：`TabBarInput / TabBarAction / TabBarState` + 全部 impl）

**`tab_bar/mod.rs`**：

```rust
//! Tab bar：拆分为 layout / render / hit / state / text 五个子模块。
//! 对外 API 通过 re-export 维持向后兼容。

pub mod layout;
pub mod render;
pub mod hit;
pub mod state;
pub mod text;

// 顶层共享类型
use shaping::Shaper;
use crate::settings::Settings;
use std::path::PathBuf;
use std::collections::HashSet;

pub struct TabBarCtx {
    pub screen_w: f32,
    pub screen_h: f32,
}

pub fn tab_bar_height() -> f32 {
    32.0 * Settings::get().dpi_scale
}

pub struct TabInfo {
    pub title: String,
    pub file_path: Option<PathBuf>,
    pub is_dirty: bool,
    pub language: String,
}

// re-export（向后兼容老代码）
pub use layout::*;
pub use render::*;
pub use hit::*;
pub use state::*;
pub use text::*;

// popup_menu 当前的 use 仍然是 `crate::tab_bar::push_quad / push_rounded_rect /
// truncate_title_by_width / TabBarCtx / TabBarLayout`，全部由上面 re-export 满足
pub use crate::popup_menu::{
    ContextMenuAction, PopupMenuAction, PopupMenuItem, PopupMenu,
    popup_menu_vertices, popup_menu_text_positions,
};
```

> ⚠️ `MouseButton` 之前定义在 `tab_bar.rs:1147`。如果你想的话，删掉这份重复（它跟 `ui::core::MouseButton` 重叠）；本阶段先**保留**老定义，避免连锁修改 `tab_bar::state::on_click(button: MouseButton)` 签名；Phase 9 收尾再合并。

- [ ] **Step 1.2：build 修复 import**

```bash
cargo build -p edit-plus-ui
```

预期：可能出现"找不到 X"的编译错误——这是因为子模块需要 `use super::*` 拿到来自其他子模块的类型。挨个修：
- `layout.rs` 需要 `use super::{TabBarCtx, TabInfo}` + `use shaping::Shaper`
- `render.rs` 需要 `use super::{TabBarCtx, layout::*}` 等
- `hit.rs` 需要 `use super::{TabBarCtx, layout::TabBarLayout}`
- `state.rs` 需要 `use super::{TabBarCtx, TabInfo, layout::*, hit::*, render::*}`
- `text.rs` 通常无依赖

修到 `cargo build` 通过。

- [ ] **Step 1.3：跑测试**

```bash
cargo test -p edit-plus-ui tab_bar
```

预期：原 tab_bar.rs 的测试都通过（拆分不改实现）。

- [ ] **Step 1.4：删除备份**

```bash
rm crates/ui/src/tab_bar/_orig.rs.bak
```

- [ ] **Step 1.5：跑 workspace**

```bash
cargo build --workspace
cargo test --workspace
```

预期：通过。`crates/app/src/app_renderer.rs` 与 `app.rs` 中所有 `ui::tab_bar::*` 引用应该照常工作（mod.rs 已 re-export）。

- [ ] **Step 1.6：提交**

```bash
git add crates/ui/src/tab_bar/ crates/ui/src/lib.rs
git rm crates/ui/src/tab_bar.rs 2>/dev/null || true
git commit -m "refactor(ui): tab_bar.rs 拆分为 mod/layout/render/hit/state/text"
```

---

## Task 2：在 widgets 子目录新增 TabBarWidget 薄壳

**Files:**
- Create: `crates/ui/src/widgets/tab_bar.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

- [ ] **Step 2.1：实现薄壳**

`TabBarWidget` 持有 `TabBarState`；`set_rect / paint / hit / on_event` 全部委托给 state 现有方法。**注意**：现有 `state::vertices()` 输出 `Vec<GlyphVertex>` 是 NDC 形态；为不破坏老路径，这一阶段我们让 widget 输出 DrawList，但实现的是"产生 GlyphVertex 后翻译回 DrawCmd::FillRect" —— 这条路径丑陋但能并存；Phase 7 再做 Layout 改 px。

**简化方案**：本任务的 widget 暂时不实现真 paint —— 只先做 hit + on_event，paint 留 stub 返回空。**真 paint 在 Task 3 完成（state 改 to_drawlist 之后）**。

```rust
//! TabBarWidget — Phase 6 阶段薄壳：只接管 hit-test + on_event 路径；
//! paint 留空，老 app_renderer 仍走 vertices() 路径。
//! Task 3 完成 state.to_drawlist 后切到真 paint。

use std::any::Any;

use crate::core::{Widget, Rect, LayoutCtx, PaintCtx, EventCtx, Event, MouseButton as CoreMB};
use crate::tab_bar::state::{TabBarState, TabBarAction, TabBarInput};

pub struct TabBarWidget {
    rect: Rect,
    state: TabBarState,
    /// Phase 6 过渡：是否已切换到 widget paint 路径
    use_widget_paint: bool,
}

impl TabBarWidget {
    pub fn new() -> Self {
        Self { rect: Rect::ZERO, state: TabBarState::new(), use_widget_paint: false }
    }

    pub fn enable_widget_paint(&mut self) { self.use_widget_paint = true; }

    pub fn state(&self) -> &TabBarState { &self.state }
    pub fn state_mut(&mut self) -> &mut TabBarState { &mut self.state }

    pub fn rect(&self) -> Rect { self.rect }
}

impl Widget for TabBarWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn paint(&self, _ctx: &mut PaintCtx) {
        // Phase 6 Task 2：留空。Task 3 完成后改为 self.state.to_drawlist(ctx)
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, ev: &Event, _ctx: &mut EventCtx) -> Option<Box<dyn Any>> {
        // 内部仍调用 state.on_click / on_mouse_move，需要 screen_w/_h；
        // widget 没有这些字段——但我们有 self.rect。tab_bar 的旧 hit_test 内部
        // 把屏幕用作 NDC 的归一化。Phase 6 阶段，旧 state 路径无法直接用 px 调用，
        // 所以本任务只把 widget 当成"事件壳"，下面我们把 on_event 同样留空，
        // 等 Phase 6 Task 4 完成 hit 改 px 之后，再实现 on_event 转译。
        //
        // 这种做法的代价：本任务结束时，事件还走老 events.rs 路径；UI 行为不变。
        let _ = ev;
        None
    }
}
```

修改 `crates/ui/src/widgets/mod.rs` 追加 `pub mod tab_bar;`。

修改 `crates/ui/src/lib.rs` 追加 `pub use widgets::tab_bar::TabBarWidget;`。

- [ ] **Step 2.2：build**

```bash
cargo build --workspace
```

预期：通过。

- [ ] **Step 2.3：提交**

```bash
git add crates/ui/src/widgets/tab_bar.rs crates/ui/src/widgets/mod.rs crates/ui/src/lib.rs
git commit -m "feat(ui-widgets): tab_bar — 薄壳 widget（仅持 state，paint/event 待落地）"
```

---

## Task 3：state 改造——把 layout 矩形从 NDC 改 Rect(px)，并加 to_drawlist

这是最重的一步。改动放在 `tab_bar/layout.rs` 与 `tab_bar/render.rs`，对老 `vertices()` 接口保持等价。

**Files:**
- Modify: `crates/ui/src/tab_bar/layout.rs`
- Modify: `crates/ui/src/tab_bar/render.rs`
- Modify: `crates/ui/src/tab_bar/state.rs`

- [ ] **Step 3.1：layout 增加 px 字段**

读 `tab_bar/layout.rs::TabEntry::rect`（NDC 形态 `[f32; 4]`）。**新增** 一个并行字段 `rect_px: Rect`：

```rust
use crate::core::Rect;

#[derive(Debug, Clone)]
pub struct TabEntry {
    pub index: usize,
    pub title: String,
    pub indicator: TabIndicator,
    pub disambiguation: Option<String>,
    pub pinned: bool,
    pub preview: bool,
    /// NDC（保留供老 vertices 路径用）
    pub rect: [f32; 4],
    /// Rect(px)（新增，供 widget paint / hit 用）
    pub rect_px: Rect,
    pub close_rect: [f32; 4],
    pub close_rect_px: Rect,
}
```

`TabBarLayout` 同样增加：

```rust
pub struct TabBarLayout {
    // ...原字段保留...
    /// 新增：所有 px 形态矩形
    pub clip_left_px: f32,
    pub clip_right_px: f32,
    pub new_tab_rect_px: Rect,
    pub overflow_left_rect_px: Rect,
    pub overflow_right_rect_px: Rect,
    pub fade_left_rect_px: Rect,
    pub fade_right_rect_px: Rect,
    pub dropdown_rect_px: Rect,
}
```

`layout_tabs` 函数内部，每次给 `entry.rect = [...]` 之后追加：

```rust
// 由 NDC 反算 px（screen_w/_h 在 ctx 里）
entry.rect_px = ndc_rect_to_px(entry.rect, ctx);
entry.close_rect_px = ndc_rect_to_px(entry.close_rect, ctx);
```

文件底部加助手：

```rust
fn ndc_rect_to_px(ndc: [f32; 4], ctx: &TabBarCtx) -> Rect {
    let [l, r, t, b] = ndc;
    let x = (l + 1.0) * 0.5 * ctx.screen_w;
    let right = (r + 1.0) * 0.5 * ctx.screen_w;
    let top = (1.0 - t) * 0.5 * ctx.screen_h;
    let bottom = (1.0 - b) * 0.5 * ctx.screen_h;
    Rect::new(x, top, (right - x).max(0.0), (bottom - top).max(0.0))
}
```

> ⚠️ 这是**临时双轨**：老 vertices 路径继续读 NDC `rect`，widget paint 路径读 `rect_px`。Phase 9 收尾时删除 NDC 字段。

- [ ] **Step 3.2：state 增加 `to_drawlist` 方法**

读 `tab_bar/state.rs::TabBarState::vertices`。在 impl 块底部追加：

```rust
use crate::core::{DrawList, PaintCtx};

impl TabBarState {
    /// Phase 6：输出 DrawList，等价于老 `vertices()` 但是 PaintCmd 形式。
    /// 内部沿用老 layout 数据；过渡期间老 vertices() 也保留。
    pub fn to_drawlist(
        &self,
        active_index: Option<usize>,
        ctx: &mut PaintCtx,
    ) {
        let Some(layout) = self.layout.as_ref() else { return; };

        // 1) tab bar 背景条
        // 老代码靠 push_quad；px 化版本：rect 占满 screen 顶部
        // 我们没有"完整 bar 矩形"——它在 layout_tabs 里隐式写死。先简化：
        // 用第一个 tab.rect_px 的 top 作为 bar.top + tab_bar_height 高度。
        if let Some(first_tab) = layout.tabs.first() {
            let bar_rect = Rect::new(
                0.0,
                first_tab.rect_px.y,
                ctx_screen_w_from_first_tab(first_tab.rect_px, layout),
                first_tab.rect_px.h,
            );
            ctx.list.fill(bar_rect, ctx.theme.tab_bar_bg);
        }

        // 2) 每个 tab
        let active = active_index.unwrap_or(0);
        for entry in &layout.tabs {
            let bg = if entry.index == active {
                ctx.theme.background
            } else {
                ctx.theme.tab_bar_bg
            };
            ctx.list.fill(entry.rect_px, bg);
            // 关闭按钮、indicator、悬浮态、预览样式…（按需扩展）
            // **Phase 6 简化**：先只画 bg + close_rect 占位。文字由 paint_backend 补。
        }

        // 3) "+"  按钮 / 滚动箭头 / 下拉 等：直接 fill bar_bg（与老视觉对齐）
        ctx.list.fill(layout.new_tab_rect_px,        ctx.theme.tab_bar_bg);
        ctx.list.fill(layout.overflow_left_rect_px,  ctx.theme.tab_bar_bg);
        ctx.list.fill(layout.overflow_right_rect_px, ctx.theme.tab_bar_bg);
        ctx.list.fill(layout.dropdown_rect_px,       ctx.theme.tab_bar_bg);

        // 4) tab 标题文本（要的字号 + 颜色）
        let font_size = 15.0 * ctx.dpi;
        for entry in &layout.tabs {
            let color = if entry.index == active {
                ctx.theme.foreground
            } else {
                let mut c = ctx.theme.foreground;
                c[0] *= 0.48; c[1] *= 0.48; c[2] *= 0.48;
                c
            };
            // 与老 tab_text_vertices 对齐：内边距 8*dpi，垂直居中
            let pad_left = 8.0 * ctx.dpi;
            let baseline = entry.rect_px.y + entry.rect_px.h * 0.5 + font_size * 0.35;
            ctx.list.text(entry.rect_px.x + pad_left, baseline, font_size, color, &entry.title);
        }
    }
}

fn ctx_screen_w_from_first_tab(_first: Rect, _layout: &TabBarLayout) -> f32 {
    // Phase 6 简化：tab 区横跨整个屏幕宽度；先返回一个大数，让 fill 到屏幕右边缘。
    // 真实正确的"bar 宽度"应该来自 ctx 的 screen_w，但 PaintCtx 不持 screen——
    // **解法**：在 TabBarLayout 增加 pub bar_rect_px: Rect 字段，由 layout_tabs 直接算好。
    // 留作 Step 3.3 修。
    9999.0
}
```

- [ ] **Step 3.3：在 layout 里把 bar_rect_px 算上**

`tab_bar/layout.rs::TabBarLayout` 加字段：

```rust
pub bar_rect_px: Rect,
```

`layout_tabs` 计算后填：

```rust
let bar_rect_px = Rect::new(0.0, 0.0, ctx.screen_w, tab_bar_h);
// 写入 layout
```

`state.rs::to_drawlist` 改用 `layout.bar_rect_px`，删除上面的 `ctx_screen_w_from_first_tab` 助手。

- [ ] **Step 3.4：跑 tab_bar 测试**

```bash
cargo test -p edit-plus-ui tab_bar
```

预期：通过（rect_px 与 bar_rect_px 不破坏老测试）。

- [ ] **Step 3.5：提交**

```bash
git add crates/ui/src/tab_bar/
git commit -m "feat(ui-tab_bar): layout 增加 px 字段；state 增加 to_drawlist"
```

---

## Task 4：TabBarWidget 真 paint + hit 切到 px

**Files:**
- Modify: `crates/ui/src/widgets/tab_bar.rs`
- Modify: `crates/ui/src/tab_bar/state.rs`（hit_test_px / on_click_px 新增）

- [ ] **Step 4.1：state 加 px 形态的 hit 与 click**

读 `tab_bar/state.rs::TabBarState::hit_test_at` / `on_click` / `on_mouse_move`。新增并行 px 版本：

```rust
impl TabBarState {
    pub fn hit_test_px(&self, px: f32, py: f32) -> Option<crate::tab_bar::hit::TabHit> {
        let layout = self.layout.as_ref()?;
        for entry in &layout.tabs {
            if entry.close_rect_px.contains(px, py) {
                return Some(crate::tab_bar::hit::TabHit::Close(entry.index));
            }
            if entry.rect_px.contains(px, py) {
                return Some(crate::tab_bar::hit::TabHit::Tab(entry.index));
            }
        }
        if layout.new_tab_rect_px.contains(px, py) {
            return Some(crate::tab_bar::hit::TabHit::NewTab);
        }
        if layout.overflow_left_rect_px.contains(px, py) {
            return Some(crate::tab_bar::hit::TabHit::ScrollLeft);
        }
        if layout.overflow_right_rect_px.contains(px, py) {
            return Some(crate::tab_bar::hit::TabHit::ScrollRight);
        }
        if layout.dropdown_rect_px.contains(px, py) {
            return Some(crate::tab_bar::hit::TabHit::Dropdown);
        }
        None
    }

    pub fn on_click_px(
        &mut self,
        px: f32, py: f32,
        button: crate::core::MouseButton,
    ) -> Option<TabBarAction> {
        let hit = self.hit_test_px(px, py)?;
        use crate::tab_bar::hit::TabHit;
        match hit {
            TabHit::Tab(idx) => {
                if button == crate::core::MouseButton::Right {
                    Some(TabBarAction::OpenContextMenu {
                        tab_index: idx,
                        anchor: [px, py], // px 形式；popup widget 在 Phase 8 处理
                    })
                } else {
                    Some(TabBarAction::SwitchTab(idx))
                }
            }
            TabHit::Close(idx)   => Some(TabBarAction::CloseTab(idx)),
            TabHit::NewTab       => Some(TabBarAction::NewEmptyTab),
            TabHit::ScrollLeft   => Some(TabBarAction::ScrollLeft),
            TabHit::ScrollRight  => Some(TabBarAction::ScrollRight),
            TabHit::Dropdown     => Some(TabBarAction::OpenOverflowMenu),
        }
    }

    pub fn on_mouse_move_px(&mut self, px: f32, py: f32) {
        self.hovered_index = self.layout.as_ref().and_then(|layout| {
            layout.tabs.iter().find(|entry| entry.rect_px.contains(px, py))
                .map(|e| e.index)
        });
    }
}
```

> ⚠️ `TabBarAction::OpenContextMenu::anchor` 的语义改成 px 后，老调用 `events.rs / app.rs` 那边也要跟着改。Phase 6 不动 popup 渲染（Phase 8 才接），但 anchor 已经从 NDC 变 px——记下来，调用方要看 `[f32; 2]` 的字段如何使用。如果旧路径里直接用作 popup 锚点（NDC 形式），那这次的语义改变会让 popup 错位。**保险起见**：保留老 `OpenContextMenu { anchor: [f32; 2] }` 是 NDC，新增 `OpenContextMenuPx { anchor_px: [f32; 2] }` 变体，widget 用新的；老路径继续用旧的，Phase 8 再统一。

按"保险起见"：在 `TabBarAction` 加一条新变体 `OpenContextMenuPx { tab_index: usize, anchor_px: [f32; 2] }`；on_click_px 返回新变体；老 on_click 继续返回 NDC 老变体。

- [ ] **Step 4.2：widget 实现真 paint + on_event**

把 `TabBarWidget::paint` 改为：

```rust
fn paint(&self, ctx: &mut PaintCtx) {
    if !self.use_widget_paint { return; }
    self.state.to_drawlist(/*active_index 由 app 通过 set_active 注入；
                              这里假设 state 自己持有它（没有的话本步加）*/, ctx);
}
```

> ⚠️ `TabBarState` 没有内置 `active_index` 字段，老路径每次从 `TabBarInput` 传入。最简的处理：在 widget 里加 `active_index_cache: Option<usize>`，由 app 通过 `set_active(...)` 注入。

加 `set_active` 与字段：

```rust
pub struct TabBarWidget {
    rect: Rect,
    state: TabBarState,
    use_widget_paint: bool,
    active_index: Option<usize>,
}

impl TabBarWidget {
    pub fn set_active(&mut self, idx: Option<usize>) { self.active_index = idx; }
}

impl Widget for TabBarWidget {
    fn paint(&self, ctx: &mut PaintCtx) {
        if !self.use_widget_paint { return; }
        self.state.to_drawlist(self.active_index, ctx);
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, ev: &Event, _ctx: &mut EventCtx) -> Option<Box<dyn Any>> {
        match ev {
            Event::MouseDown { px, py, button } => {
                let mb = match button {
                    crate::core::MouseButton::Left   => crate::core::MouseButton::Left,
                    crate::core::MouseButton::Right  => crate::core::MouseButton::Right,
                    crate::core::MouseButton::Middle => crate::core::MouseButton::Middle,
                };
                self.state.on_click_px(*px, *py, mb)
                    .map(|a| Box::new(a) as Box<dyn Any>)
            }
            Event::MouseMove { px, py } => {
                self.state.on_mouse_move_px(*px, *py);
                None
            }
            _ => None,
        }
    }
}
```

- [ ] **Step 4.3：跑测试**

```bash
cargo build --workspace
cargo test --workspace
```

预期：通过。本任务完成后 widget 仍然 `use_widget_paint = false` 默认值；老路径仍跑——下一任务才"翻开关"。

- [ ] **Step 4.4：提交**

```bash
git add crates/ui/src/widgets/tab_bar.rs crates/ui/src/tab_bar/state.rs
git commit -m "feat(ui): TabBarWidget — px paint + hit_test_px + on_click_px"
```

---

## Task 5：在 app 端把 tab_bar 切到 widget 路径

**Files:**
- Modify: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/app.rs`

- [ ] **Step 5.1：UiShell 注册真 widget**

`ui_shell.rs::idx_tabs` 注册行替换：

```rust
let idx_tabs = {
    let idx = dock.children.len();
    let t_const = 0.0_f32;
    let mut w = TabBarWidget::new();
    w.enable_widget_paint();
    dock.push(DockChild::top(w, move |_, _| t_const));
    idx
};
```

新增 `set_tabs_input` 方法，把 `TabBarInput` 转给 widget 的 state：

```rust
impl UiShell {
    pub fn set_tabs_input(
        &mut self,
        input: ui::tab_bar::state::TabBarInput<'_>,
        active_index: Option<usize>,
        shaper: Option<&mut shaping::Shaper>,
    ) {
        let any = self.dock.children[self.idx_tabs].widget.as_any_mut();
        if let Some(w) = any.downcast_mut::<ui::widgets::tab_bar::TabBarWidget>() {
            w.set_active(active_index);
            w.state_mut().update_layout(&input, shaper);
        }
    }
}
```

- [ ] **Step 5.2：app_renderer 接入 + 删 tab_text_vertices**

读 `app_renderer.rs::render` 中 Phase 5 末态。

把现有的 `self.workspace.tab_bar_state.update_layout(...)` 调用替换为通过 ui_shell：

```rust
match Settings::get_static().view_mode {
    ui::view_mode::ViewMode::Tabs => {
        if self.workspace.tick_scroll_animation() { self.needs_redraw = true; }
        let input = ui::tab_bar::state::TabBarInput {
            tabs: &tab_infos,
            active_index: Some(self.workspace.active_index),
            pinned_indices: &self.workspace.pinned_indices,
            back_enabled: !self.workspace.back_history.is_empty(),
            forward_enabled: !self.workspace.forward_history.is_empty(),
            screen_w, screen_h,
        };
        let shaper = self.text.as_mut().map(|t| &mut t.shaper);
        self.ui_shell.set_tabs_input(input, Some(self.workspace.active_index), shaper);
    }
    // sidebar 分支不变
}
```

**删除**：

```rust
vertices.extend(self.workspace.tab_bar_state.vertices(...));
vertices.extend(self.tab_text_vertices(screen_w, screen_h));
```

`app_renderer.rs::tab_text_vertices` 函数（230 行）整个删除。

- [ ] **Step 5.3：events.rs / app.rs：tab 鼠标事件改走 widget**

读 `events.rs::handle_mouse_input_left/right` / `handle_cursor_moved` 中所有 `tab_bar_state.on_click / on_mouse_move` 调用。

替换：

```rust
// Phase 6：tab_bar 鼠标事件走 ui_shell.dispatch
{
    use ui::tab_bar::state::TabBarAction;
    let ev = ui::core::Event::MouseDown {
        px, py,
        button: match button {
            winit::event::MouseButton::Left   => ui::core::MouseButton::Left,
            winit::event::MouseButton::Right  => ui::core::MouseButton::Right,
            winit::event::MouseButton::Middle => ui::core::MouseButton::Middle,
            _ => ui::core::MouseButton::Left,
        },
    };
    if let Some(boxed) = app.ui_shell.dispatch(&ev, &app.current_theme, ui::settings::Settings::get().dpi_scale) {
        if let Ok(typed) = boxed.downcast::<TabBarAction>() {
            actions.push(translate_tab_action(*typed));
            return actions;
        }
    }
}
```

新增 `events.rs::translate_tab_action`：

```rust
fn translate_tab_action(action: ui::tab_bar::state::TabBarAction) -> AppAction {
    use ui::tab_bar::state::TabBarAction as TA;
    match action {
        TA::SwitchTab(i)         => AppAction::SwitchTab(i),
        TA::CloseTab(i)          => AppAction::CloseTab(i),
        TA::NewEmptyTab          => AppAction::NewEmptyTab,
        TA::NavigateBack         => AppAction::NavigateBack,
        TA::NavigateForward      => AppAction::NavigateForward,
        TA::OpenContextMenuPx { tab_index, anchor_px } => {
            AppAction::OpenContextMenuPx { tab_index, anchor_px }
        }
        TA::OpenContextMenu { tab_index, anchor }
            => AppAction::OpenContextMenu(tab_index, anchor),
        TA::OpenOverflowMenu     => AppAction::OpenPopupOverflow,
        TA::Context { action, tab_index } =>
            AppAction::ExecuteContextMenuAction(action, tab_index),
        TA::ScrollLeft           => AppAction::ScrollTabLeft,
        TA::ScrollRight          => AppAction::ScrollTabRight,
    }
}
```

> ⚠️ `AppAction::OpenContextMenuPx` 是新枚举项（actions.rs 加一条）。Phase 8 popup 接入时用得到。如果 actions.rs 改起来麻烦，先把 px 重新转 NDC（`anchor_ndc = px_to_ndc(...)`）走老 `OpenContextMenu` —— 临时方案。

- [ ] **Step 5.4：删 workspace.tab_bar_state 字段或保留？**

Phase 6 末态有两份 TabBarState：app 老路径还在用 `workspace.tab_bar_state`；widget 内部又有一份。**Phase 6 不删 workspace 那份**——它的访问点（preview_index、scroll_offset、open_overflow_menu 等）非常多，强行删会引爆。**Phase 9 收尾时统一**。

短期措施：每帧 `set_tabs_input` 后，把 widget 内部 state 当作"权威"；workspace.tab_bar_state 当作"老路径补丁"，仍每帧 `update_layout` 一次（保证老路径如果还在用 layout 数据，不会拿到 stale）。代价：每帧 layout 算两次。Phase 9 统一时砍。

- [ ] **Step 5.5：build && run**

```bash
cargo build --workspace
cargo test --workspace
cargo run -p edit-plus-app -- README.md
```

进入应用：开多个 tab、切换、关闭、右键菜单（仍走老 popup 路径）、+ 新建、左右滚动箭头、下拉。逐个验证。

- [ ] **Step 5.6：分提交**

```bash
git add crates/app/src/ui_shell.rs
git commit -m "feat(app): ui_shell — tab_bar 真 widget 注册 + set_tabs_input"

git add crates/app/src/app_renderer.rs
git commit -m "refactor(app): tab_bar 走 widget 路径，删 tab_text_vertices(230 行)"

git add crates/app/src/events.rs crates/app/src/app.rs crates/app/src/actions.rs
git commit -m "refactor(app): tab_bar 鼠标事件路由到 widget"
```

---

## Task 6：Phase 6 收尾

- [ ] **Step 6.1：grep**

```bash
grep -rn "tab_text_vertices" crates/
grep -n "tab_bar::state::vertices\|TabBarState::vertices" crates/
```

`tab_text_vertices` 应已无命中。`vertices()` 方法仍会保留在 state.rs 里（老 API）—— Phase 9 收尾时跟着 NDC `rect` 字段一起删。

- [ ] **Step 6.2：手测要点**

- 单文档 → tab bar 隐藏（`tabs_visible = false`）
- 多文档：切换 / hover / 关闭 / 滚动 / + 新建 / 下拉
- 右键菜单：仍走老 popup（Phase 8 才接 widget）；只验证不闪退
- DPI 切换不串位

- [ ] **Step 6.3：spec 追加**

```markdown
## Phase 6 完工记录

- 拆分：tab_bar.rs(1656) → tab_bar/{mod, layout, render, hit, state, text}
- 接入：TabBarWidget；删 tab_text_vertices
- 双轨：layout 同时持 NDC + rect_px；workspace.tab_bar_state 与 widget state 双跑
- 后续：Phase 7 接 sidebar
```

```bash
git add docs/superpowers/specs/2026-06-11-ui-skeleton-design.md
git commit -m "docs(spec): UI 骨架 Phase 6 完工记录"
```

---

## 边界情况清单

1. **show_tabs=false（单文档）**：`tabs_visible=false` → dock 跳过；widget 也不参与 paint/hit。
2. **滚动动画**：`tick_scroll_animation` 与 `set_tabs_input` 顺序保持原样——动画跑在 workspace，set_tabs_input 之前调用。
3. **预览 tab（preview_index）**：`set_preview_tab` 老逻辑保留在 layout.rs；widget 通过 `state.set_preview_index` 注入。
4. **DPI 切换**：set_rect 走 LayoutCtx.dpi；layout_tabs 内部用 Settings::get().dpi_scale —— 阶段 9 才彻底改成 ctx.dpi（Phase 6 不动以减小爆炸面）。
5. **CJK 标题截断**：`truncate_title_by_width` 仍用估算路径；与老路径一致。
6. **右键菜单**：本阶段 anchor 从 NDC 改 px 时新增 `OpenContextMenuPx`；老路径仍在；Phase 8 统一。
7. **dispatch 命中后老 events 路径不再走**：⚠️ 若发现 widget 没接管的 tab 事件（比如 middle-click 关 tab？），dispatch 返回 None，事件会落到 fill（editor），不被处理。Phase 6 接受此现状；Phase 9 把所有 tab 事件都覆盖。
