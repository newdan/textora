# UI 骨架 Phase 4：search_bar widget 化（首个键盘+显隐 widget）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `ui::search_bar` 改造为 `SearchBarWidget`：接管"搜索面板"的背景、文字（query 文本 + 计数）、光标条的绘制；把 `app_renderer.rs` 中现有的搜索栏渲染分支整段删除。
**键盘转发**：search 显示时 `Esc / Enter / Backspace / 普通字符` 由 shell 路由到 widget，widget 通过 `SearchBarAction` 上行；shell 维护 `keyboard_focus = Some(SearchBar)`。这一阶段开始引入 `keyboard_focus` 概念但不重构整个键盘路径——只在已有 search 路径上做转译。

**Architecture:**
- `SearchBarWidget` 持有 `SearchBarState { query, match_count, current_match, visible }`；`set_input`/`set_visible` 由 app 注入。
- `paint`：背景 fill + query 文字 + 计数文字 + 光标条 fill。所有几何在 widget 内部用 px 计算；不再调用 `ui::search_bar::SEARCH_BAR_HEIGHT` 之外的常量。
- `on_event(KeyDown)` 返回 `SearchBarAction::{InsertChar, Backspace, Next, Prev, Close}`；app 层 downcast 后照旧调 `dv.search_state` 的方法。
- `UiShell` 增加 `keyboard_focus: Option<FocusTarget>` + `forward_key(key)` 方法；FocusTarget 暂只两枚举：`Editor / SearchBar`。
- 老 `ui::search_bar` 模块（裸函数）保留半步：仅 `SEARCH_BAR_HEIGHT` 常量保留作为 thickness 入参；其他函数 (`search_bar_bg_vertices` / `search_bar_cursor_vertices`) 删除。

**Tech Stack:** Rust 2024 · 已有 `SearchBarWidget` 路径基于 Phase 3 落地的 `paint_backend` Text 路径。

**Spec：** `docs/superpowers/specs/2026-06-11-ui-skeleton-design.md` §5、§7（阶段 4）、§8.7

---

## 文件结构

| 文件 | 改动类型 | 备注 |
|---|---|---|
| `crates/ui/src/widgets/search_bar.rs` | Create | `SearchBarWidget + SearchBarAction` |
| `crates/ui/src/widgets/mod.rs` | Modify | `pub mod search_bar;` |
| `crates/ui/src/lib.rs` | Modify | re-export |
| `crates/ui/src/search_bar.rs` | Modify | **删除** `search_bar_bg_vertices` / `search_bar_cursor_vertices`；保留 `SEARCH_BAR_HEIGHT` 常量；保留 SearchBarInput 但标记 deprecated（最终阶段 9 删） |
| `crates/app/src/ui_shell.rs` | Modify | search 位换真 widget；新增 `keyboard_focus + forward_key` |
| `crates/app/src/app_renderer.rs` | Modify | 删除原 `search_bar` 渲染分支（约 30 行） |
| `crates/app/src/app.rs` | Modify | 键盘事件分支：当 `panel_visible` 时调用 `ui_shell.forward_key`，按返回的 action 改动 `dv.search_state` |
| `crates/app/src/render_pipeline.rs` | Modify | 删除 `search_bar_text_vertices`（被 widget 取代） |

---

## Task 1：SearchBarWidget 基础

**Files:**
- Create: `crates/ui/src/widgets/search_bar.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

- [ ] **Step 1.1：实现 + 测试**

创建 `crates/ui/src/widgets/search_bar.rs`：

```rust
//! SearchBarWidget — 搜索面板的绘制 + 键盘事件转译。
//! 显隐由 app 通过 set_visible 注入（visibility 信息源是 doc.search_state.panel_visible）。
//! query 与 match_count 由 app 通过 set_input 注入。

use std::any::Any;

use crate::core::{Widget, Rect, LayoutCtx, PaintCtx, EventCtx, Event, KeyCode};

/// app 端注入的纯数据（widget 内部不知道 doc / search_state 概念）。
#[derive(Clone, Default)]
pub struct SearchBarSnapshot {
    pub query: String,
    pub match_count: usize,
    pub current_match: usize,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchBarAction {
    /// 插入一个字符到 query
    InsertChar(char),
    /// 删除最后一个字符
    Backspace,
    /// 下一个匹配
    Next,
    /// 上一个匹配
    Prev,
    /// 关闭面板
    Close,
}

pub struct SearchBarWidget {
    rect: Rect,
    snap: SearchBarSnapshot,
}

impl SearchBarWidget {
    pub fn new() -> Self {
        Self { rect: Rect::ZERO, snap: SearchBarSnapshot::default() }
    }

    pub fn set_input(&mut self, snap: SearchBarSnapshot) {
        self.snap = snap;
    }

    pub fn is_visible(&self) -> bool { self.snap.visible }
}

impl Widget for SearchBarWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 || !self.snap.visible { return; }

        // 1) 背景
        ctx.list.fill(self.rect, ctx.theme.search_bar_bg);

        let dpi = ctx.dpi;
        let font_size = 14.0 * dpi;
        let pad_left = 36.0 * dpi;
        let pad_right = 12.0 * dpi;
        let baseline = self.rect.y + self.rect.h * 0.5 + font_size * 0.35;

        // 2) query 文本
        if !self.snap.query.is_empty() {
            ctx.list.text(
                self.rect.x + pad_left,
                baseline,
                font_size,
                ctx.theme.search_bar_fg,
                &self.snap.query,
            );
        }

        // 3) 光标条（在 query 末尾）
        // 光标 x 估算：与老代码对齐 (8.0 * dpi 一字符宽)
        let cursor_x = self.rect.x + pad_left + self.snap.query.len() as f32 * 8.0 * dpi;
        let cursor_w = 2.0 * dpi;
        let cursor_h = font_size;
        let cursor_top = self.rect.y + self.rect.h * 0.5 - cursor_h * 0.4;
        ctx.list.fill(
            Rect::new(cursor_x, cursor_top, cursor_w, cursor_h * 0.8),
            ctx.theme.search_bar_fg,
        );

        // 4) 匹配计数 "n/N"，右对齐
        if self.snap.match_count > 0 {
            let s = format!("{}/{}", self.snap.current_match + 1, self.snap.match_count);
            let est_w = s.chars().count() as f32 * font_size * 0.5;
            let x = self.rect.right() - pad_right - est_w;
            ctx.list.text(x, baseline, font_size, ctx.theme.search_bar_fg, &s);
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.snap.visible && self.rect.contains(px, py)
    }

    fn on_event(&mut self, ev: &Event, _ctx: &mut EventCtx) -> Option<Box<dyn Any>> {
        if !self.snap.visible { return None; }
        match ev {
            Event::KeyDown(KeyCode::Escape)    => Some(Box::new(SearchBarAction::Close)),
            Event::KeyDown(KeyCode::Enter)     => Some(Box::new(SearchBarAction::Next)),
            Event::KeyDown(KeyCode::Backspace) => Some(Box::new(SearchBarAction::Backspace)),
            Event::KeyDown(KeyCode::Char(c))   => Some(Box::new(SearchBarAction::InsertChar(*c))),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{NoopMeasure, DrawList, DrawCmd, MouseButton};
    use crate::Theme;

    fn layout_ctx<'a>(theme: &'a Theme, m: &'a mut dyn crate::core::TextMeasure) -> LayoutCtx<'a> {
        LayoutCtx { measure: m, theme, dpi: 1.0 }
    }

    #[test]
    fn invisible_paint_emits_nothing() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = SearchBarWidget::new();
        w.set_rect(Rect::new(0.0, 32.0, 1200.0, 28.0), &mut layout);

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        assert!(list.is_empty());
    }

    #[test]
    fn visible_paint_emits_bg_and_cursor() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = SearchBarWidget::new();
        w.set_rect(Rect::new(0.0, 32.0, 1200.0, 28.0), &mut layout);
        w.set_input(SearchBarSnapshot {
            query: "".into(), match_count: 0, current_match: 0, visible: true,
        });

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        // bg + cursor (空 query 不画文本，无 count) = 2
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn visible_with_query_and_matches_emits_full_set() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = SearchBarWidget::new();
        w.set_rect(Rect::new(0.0, 32.0, 1200.0, 28.0), &mut layout);
        w.set_input(SearchBarSnapshot {
            query: "hello".into(), match_count: 7, current_match: 2, visible: true,
        });

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        // bg + query text + cursor + count text = 4
        assert_eq!(list.len(), 4);
        match &list.cmds[3] {
            DrawCmd::Text { content, .. } => assert_eq!(content, "3/7"),
            _ => panic!("expected count text"),
        }
    }

    #[test]
    fn keydown_escape_returns_close_action() {
        let theme = Theme::dark();
        let mut w = SearchBarWidget::new();
        w.set_input(SearchBarSnapshot { visible: true, ..Default::default() });
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        let action = w.on_event(&Event::KeyDown(KeyCode::Escape), &mut ctx).unwrap();
        let typed = action.downcast::<SearchBarAction>().unwrap();
        assert_eq!(*typed, SearchBarAction::Close);
    }

    #[test]
    fn keydown_char_returns_insert_char() {
        let theme = Theme::dark();
        let mut w = SearchBarWidget::new();
        w.set_input(SearchBarSnapshot { visible: true, ..Default::default() });
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        let action = w.on_event(&Event::KeyDown(KeyCode::Char('x')), &mut ctx).unwrap();
        let typed = action.downcast::<SearchBarAction>().unwrap();
        assert_eq!(*typed, SearchBarAction::InsertChar('x'));
    }

    #[test]
    fn invisible_widget_does_not_consume_keys() {
        let theme = Theme::dark();
        let mut w = SearchBarWidget::new();
        // visible=false (默认)
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        let action = w.on_event(&Event::KeyDown(KeyCode::Escape), &mut ctx);
        assert!(action.is_none());
    }

    #[test]
    fn mouse_event_in_invisible_widget_no_hit() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = SearchBarWidget::new();
        w.set_rect(Rect::new(0.0, 0.0, 100.0, 50.0), &mut layout);
        // visible=false
        assert!(!w.hit(10.0, 10.0));
    }

    #[test]
    fn enter_returns_next_for_jumping_match() {
        let theme = Theme::dark();
        let mut w = SearchBarWidget::new();
        w.set_input(SearchBarSnapshot { visible: true, ..Default::default() });
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        let action = w.on_event(&Event::KeyDown(KeyCode::Enter), &mut ctx).unwrap();
        let typed = action.downcast::<SearchBarAction>().unwrap();
        assert_eq!(*typed, SearchBarAction::Next);
    }
}
```

修改 `crates/ui/src/widgets/mod.rs`：

```rust
pub mod status_bar;
pub mod search_bar;
```

修改 `crates/ui/src/lib.rs`：在 widgets re-export 处追加：

```rust
pub use widgets::search_bar::{SearchBarWidget, SearchBarAction, SearchBarSnapshot};
```

- [ ] **Step 1.2：跑测试**

```bash
cargo test -p edit-plus-ui widgets::search_bar
```

预期：8 个测试通过。

- [ ] **Step 1.3：提交**

```bash
git add crates/ui/src/widgets/search_bar.rs crates/ui/src/widgets/mod.rs crates/ui/src/lib.rs
git commit -m "feat(ui-widgets): search_bar — 绘制 + 键盘事件转译"
```

---

## Task 2：UiShell 接 search widget + keyboard_focus

**Files:**
- Modify: `crates/app/src/ui_shell.rs`

- [ ] **Step 2.1：替换 search 占位 widget + 引入 keyboard_focus**

读 `crates/app/src/ui_shell.rs` Phase 3 末态。在 `use ui::widgets::status_bar::StatusBarWidget;` 旁追加：

```rust
use ui::widgets::search_bar::{SearchBarWidget, SearchBarSnapshot, SearchBarAction};
```

新增枚举：

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FocusTarget { Editor, SearchBar }
```

在 `UiShell` 结构里追加字段：

```rust
keyboard_focus: FocusTarget,
```

构造函数中初始化：

```rust
keyboard_focus: FocusTarget::Editor,
```

把 `idx_search` 注册行（Phase 3 后仍是 Placeholder）替换为：

```rust
let idx_search = {
    let idx = dock.children.len();
    let t_const = 0.0_f32;
    dock.push(DockChild::top(SearchBarWidget::new(), move |_, _| t_const));
    idx
};
```

新增公共方法：

```rust
impl UiShell {
    pub fn set_search_input(&mut self, snap: SearchBarSnapshot) {
        let any = self.dock.children[self.idx_search].widget.as_any_mut();
        if let Some(w) = any.downcast_mut::<SearchBarWidget>() {
            w.set_input(snap);
        }
    }

    pub fn keyboard_focus(&self) -> FocusTarget { self.keyboard_focus }
    pub fn set_keyboard_focus(&mut self, f: FocusTarget) { self.keyboard_focus = f; }

    /// 把一个键盘事件转给焦点 widget；返回 widget 给的 action（如 SearchBarAction）。
    /// 当焦点是 Editor 时直接返回 None，由 app 走老路径处理。
    pub fn forward_key(
        &mut self,
        key: ui::core::KeyCode,
        theme: &Theme,
        dpi: f32,
    ) -> Option<Box<dyn std::any::Any>> {
        let mut ctx = EventCtx { theme, dpi };
        match self.keyboard_focus {
            FocusTarget::Editor => None,
            FocusTarget::SearchBar => {
                let any = self.dock.children[self.idx_search].widget.as_any_mut();
                if let Some(w) = any.downcast_mut::<SearchBarWidget>() {
                    w.on_event(&ui::core::Event::KeyDown(key), &mut ctx)
                } else { None }
            }
        }
    }
}
```

> ⚠️ `Theme` 在文件已 import；`EventCtx` 也是。`ui::core::{KeyCode, Event}` 用 fully-qualified 路径避免与已有 import 冲突。

- [ ] **Step 2.2：测试**

在 `ui_shell.rs::tests` 模块追加：

```rust
#[test]
fn keyboard_focus_default_is_editor() {
    let shell = UiShell::new();
    assert_eq!(shell.keyboard_focus(), FocusTarget::Editor);
}

#[test]
fn forward_key_to_search_bar_returns_action() {
    let theme = Theme::dark();
    let mut m = NoopMeasure::ascii();
    let mut shell = UiShell::new();
    shell.update_frame(screen(), &theme, &mut m, &baseline_inputs());
    shell.set_search_input(SearchBarSnapshot { visible: true, ..Default::default() });
    shell.set_keyboard_focus(FocusTarget::SearchBar);

    let action = shell.forward_key(ui::core::KeyCode::Escape, &theme, 1.0).unwrap();
    let typed = action.downcast::<SearchBarAction>().unwrap();
    assert_eq!(*typed, SearchBarAction::Close);
}

#[test]
fn forward_key_to_editor_returns_none() {
    let theme = Theme::dark();
    let mut shell = UiShell::new();
    let action = shell.forward_key(ui::core::KeyCode::Char('a'), &theme, 1.0);
    assert!(action.is_none());
}
```

```bash
cargo test -p edit-plus-app ui_shell
```

预期：通过。

- [ ] **Step 2.3：提交**

```bash
git add crates/app/src/ui_shell.rs
git commit -m "feat(app): ui_shell — search widget 接入 + keyboard_focus + forward_key"
```

---

## Task 3：app 端把 search 数据塞给 widget + 键盘转发

**Files:**
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/app.rs`

- [ ] **Step 3.1：build_shell_inputs / set_search_input 串起来**

读 `app/src/app.rs::build_shell_inputs`。无需改动该函数。

读 `app/src/app_renderer.rs::render`，在 Phase 3 已有的 `set_status_input` 调用附近，追加：

```rust
// Phase 4：把 search snapshot 塞进 widget；维护 keyboard_focus
{
    use ui::widgets::search_bar::SearchBarSnapshot;
    use crate::ui_shell::FocusTarget;
    let dv = self.workspace.doc_views.get(self.workspace.active_index);
    let snap = if let Some(dv) = dv {
        SearchBarSnapshot {
            query: dv.search_state.query.clone(),
            match_count: dv.search_state.matches.len(),
            current_match: dv.search_state.active_match_idx,
            visible: dv.search_state.panel_visible,
        }
    } else {
        SearchBarSnapshot::default()
    };
    let visible = snap.visible;
    self.ui_shell.set_search_input(snap);
    self.ui_shell.set_keyboard_focus(if visible { FocusTarget::SearchBar } else { FocusTarget::Editor });
}
```

- [ ] **Step 3.2：删除老 search 渲染分支**

在 `crates/app/src/app_renderer.rs::render` 找到大约 614~653 行（Phase 3 末态行号大概仍是这附近）的：

```rust
// Search bar overlay (below tab bar, above text area)
{
    let search_visible = self.workspace.doc_views.get(self.workspace.active_index)
        .map(|dv| dv.search_state.panel_visible).unwrap_or(false);
    if search_visible {
        let search_h = ui::search_bar::SEARCH_BAR_HEIGHT * Settings::get().dpi_scale;
        let search_input = ui::search_bar::SearchBarInput { ... };
        vertices.extend(ui::search_bar::search_bar_bg_vertices(&search_input, search_h));
        vertices.extend(ui::search_bar::search_bar_cursor_vertices(&search_input, search_h));
        if !self.workspace.doc_views[self.workspace.active_index].search_state.query.is_empty()
            && let (Some(text), Some(gpu)) = (&mut self.text, &self.gpu) {
                ...
                let verts = crate::render_pipeline::search_bar_text_vertices(...);
                vertices.extend(verts);
            }
    }
}
```

**整段删除**。Phase 3 已经在 render() 末段引入了"chrome_list 走 ui_shell 渲染"的代码，search 现在自动会被 widget 化绘制。

- [ ] **Step 3.3：键盘事件分支转发**

读 `crates/app/src/app.rs:1615` 附近，处理"输入字符到 search query"的代码块。把核心动作改写为：先调 `ui_shell.forward_key(key)` 取 action，再按 action 改动 dv.search_state。

举例（伪代码，实际位置以 `events.rs::handle_keyboard` 或 `app.rs::handle_keyboard_input` 为准）：

```rust
// Phase 4：search 显示时，键盘走 widget action 路径
if self.ui_shell.keyboard_focus() == crate::ui_shell::FocusTarget::SearchBar {
    if let Some(action) = self.ui_shell.forward_key(
        translate_winit_key(key_event),  // 把 winit key event → ui::core::KeyCode
        &self.current_theme, ui::settings::Settings::get().dpi_scale,
    ) {
        if let Ok(boxed) = action.downcast::<ui::widgets::search_bar::SearchBarAction>() {
            use ui::widgets::search_bar::SearchBarAction as SA;
            if let Some(dv) = self.workspace.doc_views.get_mut(self.workspace.active_index) {
                match *boxed {
                    SA::InsertChar(c) => dv.search_state.query.push(c),
                    SA::Backspace    => { dv.search_state.query.pop(); }
                    SA::Next         => dv.search_state.next_match(),
                    SA::Prev         => dv.search_state.prev_match(),
                    SA::Close        => {
                        dv.search_state.panel_visible = false;
                        dv.search_state.clear();
                    }
                }
                self.needs_redraw = true;
            }
        }
        return; // 已处理，不再走老路径
    }
}
// ...... 老路径 (editor 文本输入) 继续 ......
```

**实现要点**：
- 在 `crates/app/src/events.rs` 新增 `translate_winit_key(key_event) -> Option<ui::core::KeyCode>`，把 winit 的 `NamedKey/Character` 翻译到我们枚举。Esc/Enter/Backspace/字符 已涵盖；其他先返回 `None`。
- 把"已经存在的 search 键盘分支"（如 `dv.search_state.query.push_str(ch)` 等）替换为通过 `forward_key` 路径。如果替换太大，**保留老路径，仅追加 widget 路径作为前置短路**：focus==SearchBar 时优先走 widget；老分支变成"被屏蔽"（实际不再走到）。Phase 9 收尾时再删老分支。

- [ ] **Step 3.4：测试**

```bash
cargo build --workspace
cargo test --workspace
cargo run -p edit-plus-app -- README.md
```

进入应用后按 ⌘F 开搜索，输入字符、Backspace、Enter 跳下一个、Esc 关。无功能回归即可。

- [ ] **Step 3.5：提交**

```bash
git add crates/app/src/app_renderer.rs crates/app/src/app.rs crates/app/src/events.rs
git commit -m "refactor(app): search_bar 走 ui_shell + forward_key；删老 vertices 分支"
```

---

## Task 4：删 ui::search_bar 老函数

**Files:**
- Modify: `crates/ui/src/search_bar.rs`
- Modify: `crates/app/src/render_pipeline.rs`（如有 `search_bar_text_vertices`）

- [ ] **Step 4.1：先 grep**

```bash
grep -rn "search_bar_bg_vertices\|search_bar_cursor_vertices\|search_bar_text_vertices" crates/
```

如果除了 `crates/ui/src/search_bar.rs` 自身和 `render_pipeline.rs` 的实现/测试外，**没有**其它调用——可直接删。

- [ ] **Step 4.2：删除函数 + 测试**

`crates/ui/src/search_bar.rs`：

- 删除 `pub fn search_bar_bg_vertices(...)`
- 删除 `pub fn search_bar_cursor_vertices(...)`
- 删除内部 `fn quad_vertices(...)`
- 删除 `#[cfg(test)] mod tests` 里所有针对这两个函数的测试
- **保留** `pub const SEARCH_BAR_HEIGHT: f32 = 28.0;`
- **保留**或删除 `pub struct SearchBarInput<'a>`（如果 grep 显示无人引用，删；否则保留）

`crates/app/src/render_pipeline.rs`：

```bash
grep -n "search_bar_text_vertices" crates/app/src/render_pipeline.rs
```

如果命中，删除整个函数定义。

- [ ] **Step 4.3：build && test**

```bash
cargo build --workspace
cargo test --workspace
```

预期：全绿。

- [ ] **Step 4.4：提交**

```bash
git add crates/ui/src/search_bar.rs crates/app/src/render_pipeline.rs
git commit -m "refactor(ui): 删 search_bar 裸函数，保留 SEARCH_BAR_HEIGHT 常量"
```

---

## Task 5：Phase 4 收尾

- [ ] **Step 5.1：手测**：⌘F 开搜索、输入中英文混合、Esc 关、再开、跨文档切换确认 search 状态正确不串台。

- [ ] **Step 5.2：grep 残余**

```bash
grep -rn "search_bar_bg_vertices\|search_bar_cursor_vertices\|search_bar_text_vertices" crates/
```

预期：无命中（含注释也清干净）。

- [ ] **Step 5.3：spec 追加完工记录**

```markdown
## Phase 4 完工记录

- 接入：SearchBarWidget；keyboard_focus + forward_key
- 删除：search_bar_bg_vertices / search_bar_cursor_vertices / search_bar_text_vertices
- 后续：Phase 5 接 scrollbar
```

```bash
git add docs/superpowers/specs/2026-06-11-ui-skeleton-design.md
git commit -m "docs(spec): UI 骨架 Phase 4 完工记录"
```

---

## 边界情况清单

1. **search 关闭瞬间**：visible=false 后 `paint` 直接 return；focus 自动回到 Editor；同时 dock thickness 走 0 → search 不吃边。
2. **跨文档切换**：每文档独立 `dv.search_state`；`build_shell_inputs / set_search_input` 每帧从 active_index 读，自动跟随。
3. **DPI 变化**：font_size = 14*dpi；光标 / 边距 / 计数都按 dpi 缩。
4. **CJK 输入**：widget 接受 `KeyCode::Char(c)`，c 直接是 char（4 字节支持）；老 `query.push_str(ch)` 改成 `push(c)` 没有功能差异。
5. **快速 Esc 双按**：第一次 Esc 关闭 search，第二次 Esc 焦点已是 Editor，老路径处理（如关 menu 等）。
6. **匹配为空时 count 不画**：`match_count > 0` 守卫。
7. **widget 不收 Enter+Shift = Prev**：当前 KeyCode 没有 modifiers；按需扩展（暂不做，留 Phase 9 改进）。简单方案：在 app 层把 Shift+Enter 翻译成 `KeyCode::Other(...)` 后由 app 直接调 `dv.search_state.prev_match()`，不走 widget。
