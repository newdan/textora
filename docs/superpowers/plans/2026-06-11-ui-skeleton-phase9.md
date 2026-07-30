# UI 骨架 Phase 9：收尾清理

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Phase 2~8 留下的"双轨"全部清理：删 NDC `[f32; 4]` 字段、删 `workspace.tab_bar_state / sidebar_state / sidebar_cfg / context_menu / overflow_menu`、删 `ui::tab_bar::state::vertices() / TabBarState::on_click（NDC 版）`、删 `ui::sidebar::SidebarState::hit_test_at（NDC 版）`、删 `ui::popup_menu::OverflowEntry` 之外的所有 NDC 残骸、合并重复 `MouseButton` 枚举、把 paint_backend FillRect 路径加圆角实现把视觉对齐回老路径。

**Architecture:** 收尾不引入新抽象；只是"剪枝"。每个 task 都很小，分批提交。

**Tech Stack:** Rust 2024。

**Spec：** `docs/superpowers/specs/2026-06-11-ui-skeleton-design.md` §10 取舍提示、§7 阶段 9

---

## Task 1：删除 ui::tab_bar 残留 NDC 字段与方法

**Files:**
- Modify: `crates/ui/src/tab_bar/{layout, state}.rs`

- [ ] **Step 1.1：删字段**

`tab_bar/layout.rs::TabEntry`：

```rust
pub struct TabEntry {
    pub index: usize,
    pub title: String,
    pub indicator: TabIndicator,
    pub disambiguation: Option<String>,
    pub pinned: bool,
    pub preview: bool,
    /// rect: [f32; 4],          ← 删
    pub rect_px: Rect,
    /// close_rect: [f32; 4],    ← 删
    pub close_rect_px: Rect,
}
```

`TabBarLayout`：

```rust
pub struct TabBarLayout {
    pub tabs: Vec<TabEntry>,
    pub overflow: bool,
    pub scroll_offset: f32,
    pub max_scroll: f32,
    pub nav_buttons: NavButtonLayout,
    /// new_tab_rect: [f32; 4],          ← 删（保留 _px）
    pub new_tab_rect_px: Rect,
    /// overflow_left_rect: [f32; 4],    ← 删（保留 _px）
    pub overflow_left_rect_px: Rect,
    pub overflow_right_rect_px: Rect,
    pub clip_left_ndc: f32,             // 暂保留：tab_bar/render.rs 老 vertices 还在用
    pub clip_right_ndc: f32,            // 同上
    /// fade_left_rect: [f32; 4],        ← 删（保留 _px）
    pub fade_left_rect_px: Rect,
    pub fade_right_rect_px: Rect,
    pub dropdown_rect_px: Rect,
    pub left_arrow_disabled: bool,
    pub right_arrow_disabled: bool,
    pub bar_rect_px: Rect,
}
```

> ⚠️ `clip_left_ndc / clip_right_ndc` 暂保留——它们仅在 `tab_bar/render.rs::tab_bar_text_positions / tab_bar_vertices` 中使用，下面 step 1.3 删除老 vertices 函数后即可清理。

`NavButtonLayout` 同样改：删 NDC 字段，保留 `_px`。

- [ ] **Step 1.2：删 layout_tabs 内部所有 NDC 计算**

读 `layout.rs::layout_tabs`，把内部所有"先算 NDC 再写入字段"的代码改为只算 px 写入 `_px`。删除：

```rust
entry.rect = ndc_rect_from_px(...);
entry.close_rect = ndc_rect_from_px(...);
```

只留 `entry.rect_px = Rect::new(...)`。

清掉 `ndc_rect_to_px` helper 和 `_ndc` 字段填充。

- [ ] **Step 1.3：删 tab_bar/render.rs 老 vertices 函数**

`tab_bar/render.rs`：

- 删 `pub fn tab_bar_vertices(...)` 整个函数
- 删 `pub fn tab_bar_text_positions(...)`
- **保留** `push_quad / push_rounded_rect`：popup_menu 已不再调；如确认无引用，全删。grep：

```bash
grep -rn "push_quad\|push_rounded_rect" crates/
```

如仅 `tab_bar/render.rs` 自身命中，删除。

- [ ] **Step 1.4：删 tab_bar/state.rs 老 vertices / on_click / hit_test_at / on_mouse_move 方法**

```rust
impl TabBarState {
    // 删 fn vertices(...)
    // 删 fn text_positions(...)
    // 删 fn hit_test_at(...)
    // 删 fn on_click(...)
    // 删 fn on_mouse_move(...)
    // 保留：hit_test_px / on_click_px / on_mouse_move_px / update_layout / set_*
    // 也删 fn max_scroll(..., screen_w, screen_h)（NDC 形态），保留 px 版（如有）
}
```

`TabBarAction::OpenContextMenu { tab_index, anchor: [f32;2] }`（NDC 版）—— 删；只留 `OpenContextMenuPx`，并把 `events.rs::translate_tab_action` 中相应分支去掉。

`pub enum MouseButton { Left, Right, Middle }`（tab_bar 版）—— 删；改为统一 `use crate::core::MouseButton`。`on_click_px` 签名 `button: crate::core::MouseButton`。

- [ ] **Step 1.5：build && test**

```bash
cargo build --workspace
cargo test --workspace
```

如果有调用方仍在引用删掉的 API（比如 `workspace.rs::update_tab_layout`），把它们改写：

- `workspace.rs::set_layout_raw / update_tab_layout / current_tab_bar_height / tick_scroll_animation` 等：本任务先**保留**这些方法（可能仍被 app 调）；只把内部对老 NDC 字段的引用迁移到 `_px`。

- [ ] **Step 1.6：提交**

```bash
git add crates/ui/src/tab_bar/
git commit -m "refactor(ui-tab_bar): 删 NDC 字段、tab_bar_vertices、老 hit/on_click"
```

---

## Task 2：删 workspace 的 UI 状态字段

**Files:**
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_renderer.rs`

- [ ] **Step 2.1：删 tab_bar_state 字段**

读 `workspace.rs:73 (tab_bar_state)`. 所有引用：

```bash
grep -rn "tab_bar_state\|workspace.tab_bar_state" crates/app/src/
```

每处分析：
- 真正读 layout 的地方 → 改读 `app.ui_shell` 内部 widget 的 state
- 写 scroll_offset / preview_index 的地方 → 移到 widget 内部
- `set_layout_raw` 调用 → 删（layout 由 widget 自己 update_layout 负责）

最简方案：在 `UiShell` 加 helper：

```rust
impl UiShell {
    pub fn tab_bar_state(&self) -> Option<&ui::tab_bar::state::TabBarState> {
        let any = (&self.dock.children[self.idx_tabs].widget).as_any();
        any.downcast_ref::<ui::widgets::tab_bar::TabBarWidget>()
            .map(|w| w.state())
    }
    pub fn tab_bar_state_mut(&mut self) -> Option<&mut ui::tab_bar::state::TabBarState> {
        let any = self.dock.children[self.idx_tabs].widget.as_any_mut();
        any.downcast_mut::<ui::widgets::tab_bar::TabBarWidget>()
            .map(|w| w.state_mut())
    }
}
```

把 `workspace.tab_bar_state` 替换为 `app.ui_shell.tab_bar_state[_mut]()`。

`workspace.rs::tab_bar_state: TabBarState` 字段删除；`workspace.rs::open_overflow_menu / set_layout_raw / new()` 中相关行删除或改写。

- [ ] **Step 2.2：删 sidebar_state / sidebar_cfg 字段**

`workspace.rs:74-75`：

```rust
pub(crate) sidebar_cfg: SidebarConfig,
pub(crate) sidebar_state: SidebarState,
```

类似 Step 2.1：通过 `ui_shell::sidebar_widget_mut() / sidebar_current_width()` 接口替代。

```rust
impl UiShell {
    pub fn sidebar_widget_mut(&mut self) -> Option<&mut ui::widgets::sidebar::SidebarWidget> {
        let any = self.dock.children[self.idx_sidebar].widget.as_any_mut();
        any.downcast_mut::<ui::widgets::sidebar::SidebarWidget>()
    }
}
```

**snapshot 持久化**：`workspace::save_snapshot` 现在读 `sidebar_cfg.pinned / width`；改读 `app.ui_shell.sidebar_widget().cfg()`。注意 snapshot 序列化字段名保持兼容（`sidebar_pinned / sidebar_width`），不破坏老 .json。

- [ ] **Step 2.3：删 context_menu / overflow_menu 字段**

`workspace.rs:70-71`：

```rust
pub(crate) context_menu: Option<PopupMenu>,
pub(crate) overflow_menu: Option<PopupMenu>,
```

直接删。所有引用：

```bash
grep -rn "context_menu\|overflow_menu" crates/app/src/
```

`app.rs::ov_clone / ctx_clone`（在 render 里 clone 用）—— 删（Phase 8 已删 popup 渲染分支后这些 clone 无意义）。

`AppAction::OpenPopupMenu(pm)` 仍保留（调用方仍用 `OpenPopupMenu(PopupMenu)` 派 action）；`OpenPopupOverflow / ClearPopupMenu` 也保留——它们都通过 ui_shell 处理。

- [ ] **Step 2.4：build && test**

```bash
cargo build --workspace
cargo test --workspace
cargo run -p edit-plus-app -- README.md
```

逐项手测：tab、sidebar、popup 全套交互不退步。

- [ ] **Step 2.5：分提交**

```bash
git add crates/app/src/workspace.rs crates/app/src/ui_shell.rs
git commit -m "refactor(app-workspace): 删 tab_bar_state/sidebar_state/sidebar_cfg/popup_menu 字段"

git add crates/app/src/app.rs crates/app/src/app_renderer.rs
git commit -m "refactor(app): 调用方改读 ui_shell 内部 widget state"
```

---

## Task 3：删 ui::sidebar 老路径 + 合并 MouseButton

**Files:**
- Modify: `crates/ui/src/sidebar.rs`
- Modify: `crates/app/src/events.rs / mouse.rs`

- [ ] **Step 3.1：删 sidebar 老 NDC**

读 `crates/ui/src/sidebar.rs`：Phase 7 已经把 layout 改 px，但可能仍有：

- 老 `pub fn hit_test_at(...screen_w, screen_h)` —— 删
- 老 `pub fn vertices` —— 已删
- `pub fn text_positions` —— 已删

确认 grep：

```bash
grep -n "fn hit_test_at\|fn vertices\|fn text_positions" crates/ui/src/sidebar.rs
```

无命中即可。

- [ ] **Step 3.2：合并 MouseButton 枚举**

```bash
grep -rn "pub enum MouseButton" crates/ui/src/
```

- `crates/ui/src/core/widget.rs` —— 保留（权威）
- `crates/ui/src/tab_bar/state.rs / hit.rs` 有重复 —— 删（task 1.4 已删）
- `crates/ui/src/sidebar.rs` 如果有 —— 删

```bash
grep -rn "tab_bar::MouseButton\|sidebar::MouseButton" crates/
```

每个引用改成 `ui::core::MouseButton`。

- [ ] **Step 3.3：events.rs winit MouseButton 翻译统一**

读 `crates/app/src/events.rs / mouse.rs`，把"winit::event::MouseButton → ui MouseButton"的翻译统一到一个 helper：

```rust
pub(crate) fn translate_mouse_button(b: winit::event::MouseButton) -> Option<ui::core::MouseButton> {
    match b {
        winit::event::MouseButton::Left   => Some(ui::core::MouseButton::Left),
        winit::event::MouseButton::Right  => Some(ui::core::MouseButton::Right),
        winit::event::MouseButton::Middle => Some(ui::core::MouseButton::Middle),
        _ => None,
    }
}
```

替换所有内联匹配。

- [ ] **Step 3.4：build && test**

```bash
cargo build --workspace
cargo test --workspace
```

- [ ] **Step 3.5：提交**

```bash
git add crates/ui/src/sidebar.rs crates/app/src/events.rs crates/app/src/mouse.rs
git commit -m "refactor(ui+app): 合并 MouseButton 到 core；统一 winit 翻译 helper"
```

---

## Task 4：删 paint_backend 残留 + 加圆角实现

**Files:**
- Modify: `crates/app/src/paint_backend.rs`

- [ ] **Step 4.1：删 drain_no_text**

Phase 2 留的 `drain_no_text` 已无人调用（Phase 3+ 全走 `drain`）。

```bash
grep -rn "drain_no_text" crates/
```

只剩定义自身。删。

- [ ] **Step 4.2：FillRect 加圆角实现**

读 `paint_backend.rs::push_quad`。改造：当 `radius > 0.0` 时，把矩形拆成"中心矩形 + 四角扇形"——简化路径：

```rust
fn push_fill(
    out: &mut Vec<GlyphVertex>,
    rect: Rect,
    color: [f32; 4],
    radius: f32,
    screen: Screen,
    clip: Option<[f32; 4]>,
) {
    if radius <= 0.5 {
        push_quad(out, rect, color, screen, clip);
        return;
    }

    let r = radius.min(rect.w * 0.5).min(rect.h * 0.5);
    // 中心 + 上下 + 左右 三段：
    // 1) 中心：高度 rect.h，宽度 rect.w - 2r
    push_quad(out, Rect::new(rect.x + r, rect.y, rect.w - 2.0 * r, rect.h), color, screen, clip);
    // 2) 左侧：宽度 r，高度 rect.h - 2r
    push_quad(out, Rect::new(rect.x, rect.y + r, r, rect.h - 2.0 * r), color, screen, clip);
    // 3) 右侧：同上
    push_quad(out, Rect::new(rect.right() - r, rect.y + r, r, rect.h - 2.0 * r), color, screen, clip);
    // 4) 四角：用 N 段三角扇近似
    push_corner(out, rect.x + r, rect.y + r, r, std::f32::consts::PI, 1.5 * std::f32::consts::PI,
                color, screen, clip);
    push_corner(out, rect.right() - r, rect.y + r, r, 1.5 * std::f32::consts::PI, 2.0 * std::f32::consts::PI,
                color, screen, clip);
    push_corner(out, rect.right() - r, rect.bottom() - r, r, 0.0, 0.5 * std::f32::consts::PI,
                color, screen, clip);
    push_corner(out, rect.x + r, rect.bottom() - r, r, 0.5 * std::f32::consts::PI, std::f32::consts::PI,
                color, screen, clip);
}

fn push_corner(
    out: &mut Vec<GlyphVertex>,
    cx: f32, cy: f32,
    r: f32,
    a0: f32, a1: f32,
    color: [f32; 4],
    screen: Screen,
    clip: Option<[f32; 4]>,
) {
    const SEGMENTS: usize = 8;
    let v_center = corner_vertex(cx, cy, color, screen, clip);
    let mut prev = corner_vertex(cx + r * a0.cos(), cy + r * a0.sin(), color, screen, clip);
    for i in 1..=SEGMENTS {
        let t = a0 + (a1 - a0) * (i as f32 / SEGMENTS as f32);
        let cur = corner_vertex(cx + r * t.cos(), cy + r * t.sin(), color, screen, clip);
        if let (Some(c), Some(p), Some(n)) = (v_center, prev, cur) {
            out.push(c); out.push(p); out.push(n);
        }
        prev = cur;
    }
}

fn corner_vertex(
    x_px: f32, y_px: f32,
    color: [f32; 4],
    screen: Screen,
    _clip: Option<[f32; 4]>,
) -> Option<GlyphVertex> {
    // 简化：暂不在圆角段做 clip（圆角通常在 popup 边缘，clip 不太相关）
    let l = x_px / screen.w * 2.0 - 1.0;
    let t = 1.0 - y_px / screen.h * 2.0;
    Some(GlyphVertex { position: [l, t], tex_coords: [0.0, 0.0], color })
}
```

`drain` 内 `DrawCmd::FillRect` 分支改调 `push_fill`。

> ⚠️ 这是一个朴素圆角实现，每角 8 段三角形 = 24 顶点；popup 4 角 = 96 顶点。性能上完全不是热点（每帧 popup 只在打开时画）。

- [ ] **Step 4.3：测试**

```bash
cargo test -p edit-plus-app paint_backend
```

新增测试：

```rust
#[test]
fn fillrect_with_radius_emits_more_vertices_than_direct_quad() {
    let mut list = DrawList::new();
    list.fill_rounded(Rect::new(0.0, 0.0, 100.0, 100.0), [1.0; 4], 8.0);
    let v = drain(&list, Screen::new(1000.0, 1000.0));
    assert!(v.len() > 6, "圆角应产生比直角更多顶点");
}
```

- [ ] **Step 4.4：手测视觉**

popup 圆角对比 git stash 上一阶段：肉眼应一致或更好。

- [ ] **Step 4.5：提交**

```bash
git add crates/app/src/paint_backend.rs
git commit -m "feat(app): paint_backend — 圆角 fill 实现 + 删 drain_no_text"
```

---

## Task 5：清理无用 AppAction 与 import

**Files:**
- Modify: `crates/app/src/actions.rs`
- Modify: `crates/app/src/app.rs`

- [ ] **Step 5.1：grep 无用 action**

```bash
grep -rn "AppAction::ScrollbarAction\|AppAction::SetScrollbarDragging\|AppAction::UpdateScrollbarState\|AppAction::EndScrollbarDrag" crates/app/src/
```

如果 dispatch 路径已经用 widget 内部 dragging 状态，这些 action 应该已经无人 push（hander 是 no-op）。**删除整组**。

- [ ] **Step 5.2：cargo clippy 清 unused import**

```bash
cargo clippy --workspace -- -W unused_imports 2>&1 | head -40
```

挨个修。

- [ ] **Step 5.3：grep 死代码**

```bash
cargo build --workspace 2>&1 | grep "warning: unused\|warning: dead_code"
```

依据警告删。

- [ ] **Step 5.4：提交**

```bash
git add crates/app/src/actions.rs crates/app/src/app.rs
git commit -m "refactor(app): 清理无用 ScrollbarAction 系列 + unused imports"
```

---

## Task 6：搬剩余 widget 文件到 widgets 目录（可选）

Phase 6~7 把 widget 包装文件放在 `crates/ui/src/widgets/`，但底层 `tab_bar / sidebar / popup_menu / scrollbar / search_bar / status_bar` 仍在 `crates/ui/src/` 平级。Phase 9 是否要把"数据/算法"层也搬进 `widgets/` 子目录？

**决策：不搬。** 因为这些底层文件本来就有"数据 + 算法 + 测试"，搬进 widgets/ 反而引发 import 路径变动。设计上**保持现状**：`crates/ui/src/{tab_bar,sidebar,popup_menu,scrollbar,search_bar,status_bar}` 提供数据 + 算法 + paint，`crates/ui/src/widgets/{...}` 提供 Widget 包装。两者都被 lib.rs re-export。

跳过此 task。

---

## Task 7：spec 完工封板 + 总览文档

- [ ] **Step 7.1：spec 末尾追加最终汇总**

```markdown
## 全部 9 个阶段完成情况

| Phase | 主要产出 | 关键 commits |
|---|---|---|
| 1 | ui::core 五件套 + measure_adapter | ... |
| 2 | UiShell + EditorHost + paint_backend 骨架 | ... |
| 3 | StatusBarWidget + Text 路径 | ... |
| 4 | SearchBarWidget + keyboard_focus | ... |
| 5 | ScrollbarWidget | ... |
| 6 | tab_bar 拆分 + Widget | ... |
| 7 | SidebarWidget | ... |
| 8 | PopupMenuWidget overlay | ... |
| 9 | 收尾：删 NDC 残骸、合并 MouseButton、加圆角 | ... |
```

- [ ] **Step 7.2：手测全套**

回归性能：在大文件上滚动、打开 sidebar、开搜索、连续打开多个 popup、跨 tab 切换 —— 不应有比 Phase 1 之前明显慢的地方（除每帧多一次 dock layout 计算 ≈ 几百纳秒，可忽略）。

- [ ] **Step 7.3：提交**

```bash
git add docs/superpowers/specs/2026-06-11-ui-skeleton-design.md
git commit -m "docs(spec): UI 骨架 9 阶段完工汇总"
```

---

## 边界情况清单（汇总）

1. **snapshot 兼容**：删 sidebar_cfg 时 snapshot 字段名 `sidebar_pinned / sidebar_width` 必须保留。
2. **deinit 顺序**：UiShell drop 时 overlays 与 dock children 各持 widget；无外部资源（GPU 资源在 app），可自然 drop。
3. **Theme 借用**：Phase 2 临时 `clone()` Theme；现仍有该 clone。Phase 9 评估是否能改成借用：app.current_theme 的所有权稳定（不会在帧内重建），借用安全；如借用冲突仍存在，保留 clone。
4. **`workspace.tab_scroll_offset / pinned_indices / back_history / forward_history`**：这些不是 UI 状态，不在 Phase 9 删除范围。
5. **search_bar input 字符**：Phase 4 留的"老路径仍在"问题——Phase 9 用 grep 确认所有 `dv.search_state.query.push_str` 都改走 widget 路径，统一掉。
