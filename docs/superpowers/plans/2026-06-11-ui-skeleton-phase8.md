# UI 骨架 Phase 8：popup_menu overlay 化

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `ui::popup_menu` 改造为 `PopupMenuWidget`，丢进 `UiShell::overlays` 而不是 `Dock::children`。删除 `app_renderer.rs::popup_menu_text_vertices`（约 80 行）和 `popup_menu_vertices`（裸函数）；popup 文字走 backend Text 路径，几何全 px。

**Architecture:**
- `popup_menu.rs` 矩形从 NDC `[f32; 4]` 改 `Rect`(px)；新增 `paint(&mut PaintCtx)`、`hit_test_px`。
- `PopupMenuWidget` 包 `PopupMenu` 数据，加 dismiss 行为（点击 widget 之外的区域关闭）。
- `UiShell::overlays: Vec<Box<dyn Widget>>` 真正派上用场——dispatch 时 overlays 优先（后入先派），未命中再下传 dock。
- `OpenContextMenu` / `OpenOverflowMenu` 老 action 改为：app 收到后构造 `PopupMenuWidget` 并 `ui_shell.push_overlay(widget)`；选项命中 → widget 返回 `PopupMenuAction`，app downcast 后执行 + `ui_shell.pop_overlay()`。

**Tech Stack:** Rust 2024 · 复用 `paint_backend` Text + Clip 路径。

**Spec：** `docs/superpowers/specs/2026-06-11-ui-skeleton-design.md` §4.7、§7（阶段 8）、§8.6

---

## 文件结构

| 文件 | 改动 |
|---|---|
| `crates/ui/src/popup_menu.rs` | layout 改 px；删 NDC `[f32; 4]` 字段；删 `popup_menu_vertices / popup_menu_text_positions` 裸函数；新增 `paint`、`hit_test_px` |
| `crates/ui/src/widgets/popup_menu.rs` | Create — `PopupMenuWidget` |
| `crates/ui/src/widgets/mod.rs` | `pub mod popup_menu;` |
| `crates/ui/src/lib.rs` | re-export |
| `crates/app/src/ui_shell.rs` | `push_overlay / pop_overlay / overlays_count` |
| `crates/app/src/app_renderer.rs` | 删 `popup_menu_text_vertices` + popup 渲染分支 |
| `crates/app/src/app.rs` | `OpenPopupMenu / OpenPopupOverflow / ClearPopupMenu` 走 overlay 路径 |

---

## Task 1：popup_menu.rs px 化

**Files:**
- Modify: `crates/ui/src/popup_menu.rs`

- [ ] **Step 1.1：改字段类型**

```rust
use crate::core::Rect;

pub struct PopupMenu {
    pub items: Vec<PopupMenuItem>,
    pub item_rects: Vec<Rect>,
    pub menu_rect: Rect,
}
```

`PopupMenu::overflow / context` 函数内部全部用 px。删除所有 `cr_x = px_ndc_x(8.0)` 之类的 NDC 工厂。直接：

```rust
pub fn overflow(
    layout: &TabBarLayoutRef, // 见下注
    dropdown_rect_px: Rect,
    active_index: usize,
    dpi: f32,
    measure: &mut dyn crate::core::TextMeasure,
) -> Self {
    let item_h = 30.0 * dpi;
    // ...
}
```

> ⚠️ 这个改造较大；`overflow` 原本依赖 `&TabBarLayout`（NDC `tabs[*].rect[1]` 这种字段读取）。Phase 6 已经在 layout 加了 `rect_px`；这里改用 px 字段。函数签名改成接受 `&[(usize, Rect /*tab rect*/, String /*title*/)]` 这样的简化数据，让 popup 不直接依赖 TabBarLayout。

简化：

```rust
pub struct OverflowEntry { pub tab_index: usize, pub title: String }

pub fn overflow_px(
    entries: &[OverflowEntry],
    dropdown_rect_px: Rect,
    screen_size: (f32, f32),
    active_index: usize,
    dpi: f32,
) -> Self { ... }
```

老 `pub fn overflow(layout: &TabBarLayout, dropdown_rect: [f32; 4], ...)` —— **删除**。调用方改用 overflow_px，自己从 `TabBarLayout::tabs` 中提取 (index, title) 列表。

`PopupMenu::context` 同款：参数从 NDC 改 px，函数名改 `context_px`。

- [ ] **Step 1.2：paint + hit_test_px**

把老 `pub fn popup_menu_vertices(menu, theme, ctx, mouse_ndc) -> Vec<GlyphVertex>` 删掉，新增 `impl PopupMenu { pub fn paint(&self, ctx: &mut PaintCtx, hovered: Option<usize>) }`：

```rust
impl PopupMenu {
    pub fn paint(&self, ctx: &mut PaintCtx, hovered: Option<usize>) {
        let radius = 8.0 * ctx.dpi;
        let shadow_offset = 3.0 * ctx.dpi;
        let shadow_alpha = ctx.theme.menu_shadow[3];

        // Shadow（直接 fill_rounded；paint_backend 暂不实现圆角，先按直角）
        let shadow_rect = Rect::new(
            self.menu_rect.x + shadow_offset,
            self.menu_rect.y + shadow_offset,
            self.menu_rect.w,
            self.menu_rect.h,
        );
        ctx.list.fill(shadow_rect, [0.0, 0.0, 0.0, shadow_alpha]);

        // Border + bg（圆角占位用直角）
        let border = 1.5 * ctx.dpi;
        let outer = Rect::new(
            self.menu_rect.x - border,
            self.menu_rect.y - border,
            self.menu_rect.w + border * 2.0,
            self.menu_rect.h + border * 2.0,
        );
        ctx.list.fill_rounded(outer, ctx.theme.menu_border, radius);
        ctx.list.fill_rounded(self.menu_rect, ctx.theme.menu_bg, radius);

        // 项
        let item_pad_x = 4.0 * ctx.dpi;
        let item_pad_y = 1.0 * ctx.dpi;
        let font_size = 14.0 * ctx.dpi;
        for (i, rect) in self.item_rects.iter().enumerate() {
            let item = &self.items[i];
            let il = rect.x + item_pad_x;
            let iw = rect.w - item_pad_x * 2.0;
            if iw <= 0.0 { continue; }

            // hover / active
            if item.is_active {
                ctx.list.fill_rounded(
                    Rect::new(il, rect.y - item_pad_y, iw, rect.h + item_pad_y * 2.0),
                    ctx.theme.menu_selected, radius * 0.5,
                );
            } else if hovered == Some(i) {
                ctx.list.fill_rounded(
                    Rect::new(il + 2.0 * ctx.dpi, rect.y + 1.0 * ctx.dpi,
                              iw - 4.0 * ctx.dpi, rect.h - 2.0 * ctx.dpi),
                    ctx.theme.menu_hover, radius * 0.5,
                );
            }

            // 文本
            let baseline = rect.y + rect.h * 0.5 + font_size * 0.35;
            ctx.list.text(
                il + 8.0 * ctx.dpi,
                baseline, font_size,
                ctx.theme.menu_text, &item.label,
            );
        }
    }

    pub fn hit_test_px(&self, px: f32, py: f32) -> Option<&PopupMenuAction> {
        for (rect, item) in self.item_rects.iter().zip(&self.items) {
            if rect.contains(px, py) {
                return Some(&item.action);
            }
        }
        None
    }
}
```

**删除**老 `popup_menu_vertices / popup_menu_text_positions` 裸函数。

- [ ] **Step 1.3：调整调用方**

popup_menu.rs 之前 `use crate::tab_bar::{push_quad, push_rounded_rect, truncate_title_by_width, ...}`——`push_quad / push_rounded_rect` 已删（不再需要 GlyphVertex），删除 import；`truncate_title_by_width` 仍在 `tab_bar/text.rs` —— 保留 import。

`tab_bar/mod.rs` 末尾 `pub use crate::popup_menu::{popup_menu_vertices, popup_menu_text_positions, ...}` —— 删除已删的两行；保留 `PopupMenu / PopupMenuItem / PopupMenuAction / ContextMenuAction` 即可。

- [ ] **Step 1.4：build & test**

```bash
cargo build --workspace
cargo test -p edit-plus-ui popup_menu
```

预期：通过（旧测试可能因签名变更要小改；逐个修）。

- [ ] **Step 1.5：提交**

```bash
git add crates/ui/src/popup_menu.rs crates/ui/src/tab_bar/mod.rs
git commit -m "refactor(ui-popup): popup_menu.rs px 化；删 NDC vertices 函数"
```

---

## Task 2：PopupMenuWidget

**Files:**
- Create: `crates/ui/src/widgets/popup_menu.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

- [ ] **Step 2.1：实现**

```rust
//! PopupMenuWidget — overlay 包装。dispatch 优先级高于 dock。

use std::any::Any;

use crate::core::{Widget, Rect, LayoutCtx, PaintCtx, EventCtx, Event, MouseButton};
use crate::popup_menu::{PopupMenu, PopupMenuAction};

/// 上行 action：要么"用户选了一项"要么"用户点了 widget 之外区域 → 关闭"。
#[derive(Debug, Clone)]
pub enum PopupOutcome {
    Selected(PopupMenuAction),
    Dismiss,
}

pub struct PopupMenuWidget {
    menu: PopupMenu,
    hovered: Option<usize>,
}

impl PopupMenuWidget {
    pub fn new(menu: PopupMenu) -> Self { Self { menu, hovered: None } }
    pub fn menu(&self) -> &PopupMenu { &self.menu }
}

impl Widget for PopupMenuWidget {
    fn set_rect(&mut self, _rect: Rect, _ctx: &mut LayoutCtx) {
        // popup 自己的位置由 PopupMenu::menu_rect 决定；忽略外面给的 rect
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        self.menu.paint(ctx, self.hovered);
    }

    fn hit(&self, _px: f32, _py: f32) -> bool {
        // overlay：永远 hit（让 dispatch 把鼠标事件先送到这里）
        true
    }

    fn on_event(&mut self, ev: &Event, _ctx: &mut EventCtx) -> Option<Box<dyn Any>> {
        match ev {
            Event::MouseMove { px, py } => {
                self.hovered = self.menu.item_rects.iter()
                    .position(|r| r.contains(*px, *py));
                None
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                if let Some(action) = self.menu.hit_test_px(*px, *py) {
                    Some(Box::new(PopupOutcome::Selected(action.clone())))
                } else {
                    // 点 widget 外（不在 menu_rect 内）→ Dismiss；
                    // 点 menu_rect 内但没命中 item（如分隔线）→ 也 Dismiss
                    Some(Box::new(PopupOutcome::Dismiss))
                }
            }
            Event::KeyDown(crate::core::KeyCode::Escape) => {
                Some(Box::new(PopupOutcome::Dismiss))
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
    use crate::popup_menu::{PopupMenuItem, ContextMenuAction};

    fn make_menu() -> PopupMenu {
        PopupMenu {
            items: vec![
                PopupMenuItem {
                    label: "Close".into(),
                    is_active: false, is_separator: false,
                    action: PopupMenuAction::Context {
                        action: ContextMenuAction::Close, tab_index: 0,
                    },
                },
            ],
            item_rects: vec![Rect::new(100.0, 100.0, 160.0, 30.0)],
            menu_rect: Rect::new(100.0, 100.0, 160.0, 30.0),
        }
    }

    #[test]
    fn paint_emits_some_commands() {
        let theme = Theme::dark();
        let w = PopupMenuWidget::new(make_menu());
        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        assert!(!list.is_empty());
    }

    #[test]
    fn click_inside_item_returns_selected() {
        let theme = Theme::dark();
        let mut w = PopupMenuWidget::new(make_menu());
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        let action = w.on_event(
            &Event::MouseDown { px: 150.0, py: 110.0, button: MouseButton::Left },
            &mut ctx,
        ).unwrap();
        let outcome = action.downcast::<PopupOutcome>().unwrap();
        assert!(matches!(*outcome, PopupOutcome::Selected(_)));
    }

    #[test]
    fn click_outside_returns_dismiss() {
        let theme = Theme::dark();
        let mut w = PopupMenuWidget::new(make_menu());
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        let action = w.on_event(
            &Event::MouseDown { px: 500.0, py: 500.0, button: MouseButton::Left },
            &mut ctx,
        ).unwrap();
        let outcome = action.downcast::<PopupOutcome>().unwrap();
        assert!(matches!(*outcome, PopupOutcome::Dismiss));
    }

    #[test]
    fn escape_returns_dismiss() {
        let theme = Theme::dark();
        let mut w = PopupMenuWidget::new(make_menu());
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        let action = w.on_event(
            &Event::KeyDown(crate::core::KeyCode::Escape),
            &mut ctx,
        ).unwrap();
        let outcome = action.downcast::<PopupOutcome>().unwrap();
        assert!(matches!(*outcome, PopupOutcome::Dismiss));
    }
}
```

- [ ] **Step 2.2：跑测试**

```bash
cargo test -p edit-plus-ui widgets::popup_menu
```

预期：4 个测试通过。

- [ ] **Step 2.3：提交**

```bash
git add crates/ui/src/widgets/popup_menu.rs crates/ui/src/widgets/mod.rs crates/ui/src/lib.rs
git commit -m "feat(ui-widgets): popup_menu — overlay widget"
```

---

## Task 3：UiShell overlays 路由

**Files:**
- Modify: `crates/app/src/ui_shell.rs`

- [ ] **Step 3.1：overlay API**

读 `ui_shell.rs::UiShell::overlays`（Phase 2 已声明为空 `Vec<Box<dyn Widget>>`）。新增方法：

```rust
impl UiShell {
    pub fn push_overlay<W: ui::core::Widget + 'static>(&mut self, widget: W) {
        self.overlays.push(Box::new(widget));
    }
    pub fn pop_overlay(&mut self) -> Option<Box<dyn ui::core::Widget>> {
        self.overlays.pop()
    }
    pub fn overlay_count(&self) -> usize { self.overlays.len() }
    pub fn clear_overlays(&mut self) { self.overlays.clear(); }

    pub fn paint_overlays(&self, theme: &Theme, dpi: f32, list: &mut ui::core::DrawList) {
        let mut ctx = ui::core::PaintCtx { list, theme, dpi };
        for ov in &self.overlays {
            ov.paint(&mut ctx);
        }
    }
}
```

修改 `paint_chrome`：

```rust
pub fn paint_chrome(&self, theme: &Theme, dpi: f32) -> DrawList {
    let mut list = DrawList::new();
    {
        let mut ctx = PaintCtx { list: &mut list, theme, dpi };
        self.dock.paint(&mut ctx);
    }
    self.paint_overlays(theme, dpi, &mut list);
    list
}
```

`dispatch` 已经在 Phase 2 写过 overlays 优先；确认仍正确：

```rust
pub fn dispatch(&mut self, ev: &Event, theme: &Theme, dpi: f32)
    -> Option<Box<dyn Any>>
{
    let mut ctx = EventCtx { theme, dpi };
    for ov in self.overlays.iter_mut().rev() {
        if let Some(action) = ov.on_event(ev, &mut ctx) {
            return Some(action);
        }
    }
    self.dock.dispatch(ev, &mut ctx)
}
```

- [ ] **Step 3.2：测试**

```rust
#[test]
fn overlay_dispatch_takes_priority_over_dock() {
    use ui::widgets::popup_menu::{PopupMenuWidget, PopupOutcome};
    use ui::popup_menu::{PopupMenu, PopupMenuItem, PopupMenuAction, ContextMenuAction};
    let theme = Theme::dark();
    let mut shell = UiShell::new();

    let menu = PopupMenu {
        items: vec![PopupMenuItem {
            label: "x".into(), is_active: false, is_separator: false,
            action: PopupMenuAction::Context { action: ContextMenuAction::Close, tab_index: 0 },
        }],
        item_rects: vec![ui::core::Rect::new(100.0, 100.0, 60.0, 30.0)],
        menu_rect: ui::core::Rect::new(100.0, 100.0, 60.0, 30.0),
    };
    shell.push_overlay(PopupMenuWidget::new(menu));

    let action = shell.dispatch(
        &ui::core::Event::MouseDown {
            px: 110.0, py: 110.0,
            button: ui::core::MouseButton::Left,
        },
        &theme, 1.0,
    ).unwrap();
    let outcome = action.downcast::<PopupOutcome>().unwrap();
    assert!(matches!(*outcome, PopupOutcome::Selected(_)));
}
```

```bash
cargo test -p edit-plus-app ui_shell
```

- [ ] **Step 3.3：提交**

```bash
git add crates/app/src/ui_shell.rs
git commit -m "feat(app): ui_shell — overlays 路由 + paint_overlays"
```

---

## Task 4：app 端把 popup 走 overlay 路径

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/workspace.rs`

- [ ] **Step 4.1：app.rs 处理 OpenPopupMenu / OpenPopupOverflow**

读 `crates/app/src/app.rs:725 / 738 / 733`。

把：

```rust
AppAction::OpenPopupMenu(pm) => { self.workspace.context_menu = Some(pm); }
```

改为：

```rust
AppAction::OpenPopupMenu(pm) => {
    self.ui_shell.clear_overlays(); // 一次只允许一个 popup
    self.ui_shell.push_overlay(ui::widgets::popup_menu::PopupMenuWidget::new(pm));
}
```

`AppAction::OpenPopupOverflow` 同款：

```rust
AppAction::OpenPopupOverflow => {
    let dropdown_rect_px = /* 从 ui_shell 拿 tab_bar widget 的 dropdown_rect_px */;
    let entries: Vec<_> = self.workspace.doc_views.iter().enumerate()
        .map(|(i, dv)| ui::popup_menu::OverflowEntry {
            tab_index: i,
            title: dv.title(),
        })
        .collect();
    let menu = ui::popup_menu::PopupMenu::overflow_px(
        &entries, dropdown_rect_px,
        (screen_w, screen_h),
        self.workspace.active_index,
        ui::settings::Settings::get().dpi_scale,
    );
    self.ui_shell.clear_overlays();
    self.ui_shell.push_overlay(ui::widgets::popup_menu::PopupMenuWidget::new(menu));
}
```

`AppAction::ClearPopupMenu`：

```rust
AppAction::ClearPopupMenu => {
    self.ui_shell.clear_overlays();
}
```

- [ ] **Step 4.2：dispatch 收到 PopupOutcome 后处理**

在 Phase 6 / 7 events.rs 鼠标 dispatch 路径里追加（位于 dock action 之前）：

```rust
{
    use ui::widgets::popup_menu::PopupOutcome;
    let ev = ui::core::Event::MouseDown { /* ... */ };
    if let Some(boxed) = app.ui_shell.dispatch(&ev, &app.current_theme,
        ui::settings::Settings::get().dpi_scale)
    {
        if let Ok(outcome) = boxed.downcast::<PopupOutcome>() {
            match *outcome {
                PopupOutcome::Selected(pm_action) => {
                    use ui::popup_menu::PopupMenuAction;
                    match pm_action {
                        PopupMenuAction::SwitchTab(i) => actions.push(AppAction::SwitchTab(i)),
                        PopupMenuAction::Context { action, tab_index } => {
                            actions.push(AppAction::ExecuteContextMenuAction(action, tab_index));
                        }
                    }
                    actions.push(AppAction::ClearPopupMenu);
                }
                PopupOutcome::Dismiss => actions.push(AppAction::ClearPopupMenu),
            }
            return actions;
        }
    }
}
```

类似地把 dispatch 的优先级 keypath 处理 KeyDown(Escape) 时如果命中 overlay 的 Dismiss 也走同款分支。

- [ ] **Step 4.3：删 app_renderer.rs popup 渲染分支 + 删 popup_menu_text_vertices**

读 `app_renderer.rs:674-692` 的 popup 顶点 extend 块：

```rust
if show_tabs {
    if let Some(ref om) = ov_clone {
        let ctx = ui::tab_bar::TabBarCtx { screen_w, screen_h };
        vertices.extend(tab_bar::popup_menu_vertices(om, &self.current_theme, &ctx, _mouse_ndc));
        vertices.extend(self.popup_menu_text_vertices(screen_w, screen_h, om));
    }
    if let Some(ref cm) = ctx_clone {
        ...
    }
}
```

**整段删除**。Phase 3 的 chrome_list 路径已经通过 `paint_overlays` 自动包含 popup。

`app_renderer.rs::popup_menu_text_vertices` 函数定义（约 14-96 行那 80 行）—— **删除**整个函数。

- [ ] **Step 4.4：删 workspace.context_menu / overflow_menu 字段（可选）**

读 `workspace.rs:70/71`：

```rust
pub(crate) context_menu: Option<PopupMenu>,
pub(crate) overflow_menu: Option<PopupMenu>,
```

如果所有引用都已经走 ui_shell.overlays，可以删。**Phase 8 不强制删**——风险大；先保留为"幻影字段"，Phase 9 收尾删。

- [ ] **Step 4.5：build && run**

```bash
cargo build --workspace
cargo test --workspace
cargo run -p edit-plus-app -- README.md
```

测试：
- 右键 tab → context menu 弹出，点击关闭/复制路径；点击外部关闭；按 Esc 关闭
- tab 太多时点下拉箭头 → overflow 菜单弹；选项切 tab；Esc 关
- sidebar 设置按钮 → settings 菜单

- [ ] **Step 4.6：分提交**

```bash
git add crates/app/src/app.rs
git commit -m "refactor(app): popup 走 ui_shell overlay 路径"

git add crates/app/src/app_renderer.rs
git commit -m "refactor(app): 删 popup_menu_text_vertices(80 行) + 老 popup 渲染分支"

git add crates/app/src/events.rs
git commit -m "refactor(app): dispatch 处理 PopupOutcome → ClearPopupMenu"
```

---

## Task 5：Phase 8 收尾

- [ ] **Step 5.1：grep**

```bash
grep -rn "popup_menu_text_vertices\|popup_menu_vertices\|popup_menu_text_positions" crates/
```

预期：仅命中 popup_menu.rs 内部已删除的痕迹（应无）。

- [ ] **Step 5.2：手测核心交互**

- 右键 tab：菜单出现位置 / 字体颜色 / hover 高亮 / 选择关闭  
- overflow 菜单：tab 多时一键看全
- ESC 关 popup 不影响 search bar 已开（如果 search 也开着）—— overlays 优先，KeyDown(Esc) 命中 overlay → Dismiss → 清 overlay。第二次 ESC 才关 search。

- [ ] **Step 5.3：spec 追加**

```markdown
## Phase 8 完工记录

- 改造：popup_menu.rs 改 px；删 popup_menu_vertices / text_positions
- 接入：PopupMenuWidget 进 UiShell::overlays
- 删除：app_renderer.rs::popup_menu_text_vertices(80 行)
- 双轨残留：workspace.context_menu / overflow_menu 字段保留（Phase 9 删）
- 后续：Phase 9 收尾
```

```bash
git add docs/superpowers/specs/2026-06-11-ui-skeleton-design.md
git commit -m "docs(spec): UI 骨架 Phase 8 完工记录"
```

---

## 边界情况清单

1. **同时打开 context + overflow**：`clear_overlays` 在 push 之前调用，保证一次只一个 popup。
2. **popup 后再点 tab 中央**：第一击 dispatch 命中 overlay 返回 `Dismiss + ClearPopupMenu`；第二击才走 dock 命中 tab。手测确认两次 click 行为正确。
3. **popup 跨屏边界**：context_px 的 clamp 逻辑保留（菜单不出屏）。
4. **键盘焦点 vs popup**：popup 优先吃 KeyDown；search 显示 + popup 显示同时存在时，popup 关后 search focus 仍在。`forward_key` 路径应在"overlays 命中" 后才回到 search 的判断；本阶段 `forward_key` 直接读 `keyboard_focus`，与 overlays 不冲突——因为 KeyDown 也走 dispatch（overlay 命中 Dismiss），不走 forward_key。这个交互路径要在手测里专门验证。
5. **圆角占位**：本阶段 paint_backend 仍画直角矩形；视觉与老路径会有细微差。如果不可接受，**Phase 9** 在 paint_backend FillRect 路径加圆角实现；本阶段先放过。
6. **shadow alpha**：用 `[0,0,0,alpha]` 直接 fill；与老多层阴影对比可能略糙。同 5，Phase 9 优化。
