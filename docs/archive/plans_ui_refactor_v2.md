> 实施完成：2026-06-12（commit ebe8cb1 ~ a7a9ef3）

# UI 架构重构计划 v2 — 相对坐标 + WidgetId 焦点路由

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让所有 widget 在自身局部坐标系内工作，并用 `WidgetId` 取代焦点路由里的硬编码 `downcast::<SearchBarWidget>()`。

**Architecture:** Dock/UiShell 在 paint 与事件分发时把累计偏移量推/弹给子 widget；widget 内部 `rect` 退化为 `(0, 0, w, h)`。键盘事件通过 `WidgetId` 树形匹配派发，废除 `FocusTarget` 枚举。保持现有扁平容器结构，零额外堆分配。

**Tech Stack:** Rust 2024、自研 UI crate（`crates/ui`）、应用层 shell（`crates/app`）、`cargo test --workspace` 作为回归。

**重要约束（来自 CLAUDE.md）：**
- 阶段尽量原子化，单阶段改动 ≤ 3 个文件即一次提交，超过则拆子阶段。
- 每次提交都要能 `cargo build` 通过；改协议要先写测试再改实现（TDD）。
- 反复修同一处超过两轮就停下来重审根因。
- 用中文回复用户；commit message 也使用中文。

---

## 0. 设计决议（写代码前先读完）

### 0.1 `WidgetId` 生成与命名

| 项 | 决策 |
|---|---|
| 类型 | `pub struct WidgetId(pub u64)`，`Copy + Eq + Hash` |
| 来源 | **手写常量**，不用运行时分配。所有 ID 集中在 `crates/ui/src/core/widget.rs` 的 `pub mod ids` 模块中，`pub const SEARCH_BAR: WidgetId = WidgetId(0x01);` 等。 |
| 唯一性 | 同类 widget 多实例的需求**目前不存在**（dock children 每类各 1 个），未来若需多实例再扩展为 `WidgetId(kind << 32 | instance)`，但本次重构**不引入**。 |
| `Widget::id()` | trait 默认 `None`；只有需要被键盘路由的 widget（SearchBar）必须 override。其余可以选择性 override 给 future-proof，但本计划只强制 SearchBar。 |

ID 列表（本次只需 1 个）：
```rust
pub mod ids {
    use super::WidgetId;
    pub const SEARCH_BAR: WidgetId = WidgetId(1);
    // 未来扩展位（不在本次实现中）：
    // pub const COMMAND_PALETTE: WidgetId = WidgetId(2);
}
```

### 0.2 `PaintCtx::offset` / `global_alpha` 推/弹规约

- `PaintCtx` 新增两个字段：
  - `pub offset: (f32, f32)`：**只读语义**，DrawList helper 在写入 `DrawCmd` 时把 offset 加到坐标上；widget 自己不修改。
  - `pub global_alpha: f32`（默认 `1.0`）：**只读语义**，预留给后续淡入淡出动画（如 sidebar HoverPeek fade）。本次重构**只加字段、不接线**——helper 与 paint_backend 暂不读取它，行为完全等价于今天。后续美化方案接入时再扩展 helper 把 alpha 乘进 color。**写入字段值是容器在 paint 子 widget 前/后的事，widget 仅读。**
- 容器（`Dock` / `UiShell`）在调用每个子 widget 的 `paint` 之前：
  ```rust
  let saved_offset = ctx.offset;
  let saved_alpha = ctx.global_alpha;
  ctx.offset = (saved_offset.0 + child_layout_rect.x, saved_offset.1 + child_layout_rect.y);
  // ctx.global_alpha 暂时保持不变（默认 1.0）；后续动画阶段在此点乘 alpha。
  child.widget.paint(ctx);
  ctx.offset = saved_offset;
  ctx.global_alpha = saved_alpha;
  ```
- DrawList helper 改造范围（**全部以"emit DrawCmd 的方法"为准**，不再列举具体方法名）：
  - `FillRect`：`rect.x += offset.0; rect.y += offset.1;`
  - `Text`：`x += offset.0; y_baseline += offset.1;`
  - `PushClip(rect)`：同 FillRect。`PopClip` 不带坐标，无需改动。
- 嵌套 widget（sidebar 内含 list）：list 自己不知道 offset 是多少，直接用相对坐标画即可，**累计 offset 在容器层管理**。Sidebar 是叶子级容器，本计划里它内部画 list 是直接调用 `self.list.paint(ctx)`，因此 sidebar 也要在调 list 之前推自己的内部偏移（list 的 layout_rect 也是相对的）。

### 0.3 容器事件分发的相对坐标转换

- `Event` 类型不变。容器在派发前把鼠标事件 `px/py` 减去子节点的 `layout_rect` 起点，构造一个新的 `Event` 再投递。
- **Capture 状态**：`is_capturing()` 的子节点照常拿到事件，但**px/py 仍然先减去其 layout_rect 起点**，可能为负数或大于 `w`。每个会 capture 的 widget 必须在 `on_event` 中容忍越界相对坐标——这是规约的一部分。
- Wheel 事件也走同样的减法（只为统一规则；具体 widget 可以只用 dy）。
- KeyDown 不带坐标，分发逻辑见 0.4。

### 0.3.1 `EventCtx::cursor_hint` 通道（B1 方案）

- **目的**：让 widget 自己声明"当前鼠标位置应该用哪个 cursor"，而不是把 cursor 决策塞到 `events.rs` 一长串 if 链里。这是后续 sidebar 美化（边缘 resize → ColResize）和未来 widget cursor 决策的共用通道。
- **协议**：
  - `EventCtx` 新增字段 `pub cursor_hint: Option<winit::window::CursorIcon>`，初值 `None`。
  - widget 在 `on_event` 处理 `MouseMove` 时，若希望覆盖默认 cursor，写入 `ctx.cursor_hint = Some(...);`。多个 widget 写入时，**后写者覆盖前者**——dispatch 顺序天然保证 capture > overlay > dock children > fill。
  - `Dock::dispatch` / `UiShell::dispatch` **不读不清** `cursor_hint`，只是把同一个 ctx 透传给所有 widget。
  - 调用方（`events.rs::handle_cursor_moved`）在 dispatch 完后读取 `ctx.cursor_hint`：若为 `Some`，push `AppAction::SetCursor(...)`；若为 `None`，走原有默认逻辑（编辑器区 → Text，title bar → Default 等）。
- **本次重构只加字段、不接线**：阶段 1 加字段并填默认值；现存 cursor 决策仍留在 `events.rs`，不强行迁移。等后续 sidebar 美化方案的"D 阶段"（resize cursor）和增量迁移其他 widget 时再用。这与 `global_alpha` 同源——先把协议槽位预留出来，避免重复扫所有 `EventCtx { ... }` 字面量。
- **为什么是 winit 类型**：`crates/ui` 已经依赖 `winit`（`theme.rs::from_winit`），不会引入新依赖。

### 0.4 键盘焦点路由 API

- `UiShell.keyboard_focus: Option<FocusTarget>` → `Option<WidgetId>`。
- 新增 trait 方法 `Widget::id(&self) -> Option<WidgetId> { None }`。
- `Dock` 不感知焦点。**焦点路由由 `UiShell::dispatch` 处理**：
  ```rust
  if let Event::KeyDown(_) = ev {
      if let Some(focus_id) = self.keyboard_focus {
          // 扫描 dock children + overlays，找到 id() == focus_id 的 widget
          if let Some(action) = self.route_key_to_focus(ev, focus_id, ctx) {
              return Some(action);
          }
      }
      // 未命中则像今天一样下沉到 dock.fill（编辑器）
  }
  ```
- 保留 `forward_key()` 接口，但实现改成调用上面的 `route_key_to_focus`，**不再 downcast**。

### 0.5 `DockChild.layout_rect` vs widget 内部 `self.rect`

- 决议：**容器持有 layout_rect（绝对/父坐标系），widget 内部 rect 在迁移完成后只保留 `w/h`（局部坐标系）**。
- 过渡期（阶段 3、4）允许 widget 仍持有 `(x, y, w, h)`，但 `x/y` 必须始终为 0；通过 grep 检查兜底。
- `Dock.fill` 同样需要 `layout_rect`：把 fill 字段从 `Box<dyn Widget>` 包成 `DockFill { widget, layout_rect }`，分发与 paint 时一起做坐标转换。

### 0.6 Overlay 容器

- `UiShell.overlays: Vec<Box<dyn Widget>>` → `Vec<OverlayEntry>`，其中 `OverlayEntry { widget, layout_rect }`。
- `push_overlay(widget, layout_rect)`：调用方必须提供 overlay 在屏幕上的矩形（popup 已在 `popup_menu.rs` 中算好）。
- 事件分发减法同 Dock。

### 0.7 测试策略与回归红线

- 阶段 1、2 是**纯协议**变动：必须先写新单元测试（offset 累计、相对坐标分发、capture-越界），让其失败，再改实现。
- 阶段 3、4 每个 widget 迁移后跑 `cargo test -p ui` + 手测脚本（见每个 widget 的 acceptance）。
- 现有断言绝对坐标的测试（`Dock` 单元测试中大量出现）按"测试什么由当前阶段决定"原则同步改：
  - 阶段 1 的"layout_rect 缓存"测试新增。
  - 阶段 2 的"分发减法"测试新增；同时把 `dock_dispatch_routes_to_topmost_hit` 等改成断言子节点收到的是相对坐标 ev。
- 任何阶段如果发现需要回退，在该阶段最末提交前回滚即可——每阶段一个独立 commit。

---

## 文件结构（受影响清单）

| 文件 | 改动类型 | 说明 |
|---|---|---|
| `crates/ui/src/core/widget.rs` | 修改 | 加 `WidgetId`、`ids` 模块、`Widget::id()`、`PaintCtx.offset` |
| `crates/ui/src/core/paint.rs` | 修改 | DrawList helper 在 emit 时叠加 ctx.offset（需要 helper 接收 offset，见阶段 1） |
| `crates/ui/src/core/dock.rs` | 修改 | `DockChild.layout_rect`、`DockFill { widget, layout_rect }`、paint/dispatch 推弹 offset |
| `crates/ui/src/widgets/search_bar.rs` | 修改 | 局部坐标 + 实现 `id() = SEARCH_BAR` |
| `crates/ui/src/widgets/status_bar.rs` | 修改 | 局部坐标 |
| `crates/ui/src/widgets/tab_bar.rs` | 修改 | 局部坐标 |
| `crates/ui/src/widgets/scrollbar.rs` | 修改 | 局部坐标（注意 capture 越界规约） |
| `crates/ui/src/widgets/list.rs` | 修改 | 局部坐标 + hit_row 重算 |
| `crates/ui/src/widgets/sidebar.rs` | 修改 | 局部坐标 + drag 公式重写 |
| `crates/ui/src/sidebar.rs` | 修改 | `SidebarLayout` 内部 rect 改局部坐标，`hit_test_px` 同步 |
| `crates/ui/src/widgets/popup_menu.rs` | 修改 | 局部坐标 |
| `crates/app/src/ui_shell.rs` | 修改 | overlays 改 OverlayEntry、`keyboard_focus: Option<WidgetId>`、消除 `forward_key` 内 downcast |

---

## 阶段 1：底层协议扩展（widget.rs / paint.rs）

**目标：** 引入 `WidgetId`、`PaintCtx.offset`、helper 自动叠加；不改任何业务 widget。

**Files:**
- Modify: `crates/ui/src/core/widget.rs`
- Modify: `crates/ui/src/core/paint.rs`

### Task 1.1 写失败测试：`PaintCtx.offset` 让 DrawList helper 输出绝对坐标

- [ ] **Step 1: 在 `crates/ui/src/core/paint.rs` 的 `tests` 模块里添加测试**

```rust
#[test]
fn fill_with_offset_translates_command_rect() {
    let mut dl = DrawList::new();
    // 模拟容器在调子 widget 前推了 offset (50, 100)
    dl.fill_with_offset(
        Rect::new(10.0, 20.0, 30.0, 40.0),
        [1.0, 0.0, 0.0, 1.0],
        (50.0, 100.0),
    );
    match &dl.cmds[0] {
        DrawCmd::FillRect { rect, .. } => {
            assert_eq!(rect.x, 60.0);
            assert_eq!(rect.y, 120.0);
            assert_eq!(rect.w, 30.0);
            assert_eq!(rect.h, 40.0);
        }
        _ => panic!("expected FillRect"),
    }
}

#[test]
fn text_with_offset_translates_baseline() {
    let mut dl = DrawList::new();
    dl.text_with_offset(5.0, 8.0, 12.0, [0.0; 4], "x", (50.0, 100.0));
    match &dl.cmds[0] {
        DrawCmd::Text { x, y_baseline, .. } => {
            assert_eq!(*x, 55.0);
            assert_eq!(*y_baseline, 108.0);
        }
        _ => panic!("expected Text"),
    }
}

#[test]
fn clip_with_offset_translates_push_only() {
    let mut dl = DrawList::new();
    dl.clip_with_offset(Rect::new(0.0, 0.0, 10.0, 10.0), (50.0, 100.0), |_| {});
    match &dl.cmds[0] {
        DrawCmd::PushClip(r) => {
            assert_eq!(r.x, 50.0);
            assert_eq!(r.y, 100.0);
        }
        _ => panic!("expected PushClip"),
    }
    assert!(matches!(dl.cmds[1], DrawCmd::PopClip));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p ui paint::tests::fill_with_offset_translates_command_rect`
Expected: 编译失败 — `fill_with_offset` 未定义。

- [ ] **Step 3: 实现 helper 函数（保留旧 helper 不动，新增 `_with_offset` 变体）**

在 `crates/ui/src/core/paint.rs` 的 `impl DrawList` 中追加：

```rust
pub fn fill_with_offset(&mut self, rect: Rect, color: [f32; 4], offset: (f32, f32)) {
    self.cmds.push(DrawCmd::FillRect {
        rect: Rect::new(rect.x + offset.0, rect.y + offset.1, rect.w, rect.h),
        color,
        radius: 0.0,
    });
}

pub fn fill_rounded_with_offset(
    &mut self,
    rect: Rect,
    color: [f32; 4],
    radius: f32,
    offset: (f32, f32),
) {
    self.cmds.push(DrawCmd::FillRect {
        rect: Rect::new(rect.x + offset.0, rect.y + offset.1, rect.w, rect.h),
        color,
        radius,
    });
}

pub fn text_with_offset(
    &mut self,
    x: f32,
    y_baseline: f32,
    font_size: f32,
    color: [f32; 4],
    s: &str,
    offset: (f32, f32),
) {
    self.cmds.push(DrawCmd::Text {
        x: x + offset.0,
        y_baseline: y_baseline + offset.1,
        font_size,
        color,
        content: s.to_string(),
    });
}

pub fn clip_with_offset<F: FnOnce(&mut DrawList)>(
    &mut self,
    rect: Rect,
    offset: (f32, f32),
    f: F,
) {
    self.cmds.push(DrawCmd::PushClip(Rect::new(
        rect.x + offset.0,
        rect.y + offset.1,
        rect.w,
        rect.h,
    )));
    f(self);
    self.cmds.push(DrawCmd::PopClip);
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p ui paint::tests`
Expected: 全部通过。

### Task 1.2 写失败测试：`Widget::id()` 默认 None，可被 override

- [ ] **Step 1: 在 `crates/ui/src/core/widget.rs` 的 `tests` 模块尾部追加测试**

```rust
#[test]
fn widget_id_default_is_none() {
    struct Anon;
    impl Widget for Anon {
        fn set_rect(&mut self, _: Rect, _: &mut LayoutCtx) {}
        fn paint(&self, _: &mut PaintCtx) {}
        fn hit(&self, _: f32, _: f32) -> bool { false }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }
    let w = Anon;
    assert!(w.id().is_none());
}

#[test]
fn widget_id_can_be_overridden() {
    use crate::core::widget::ids;
    struct Named;
    impl Widget for Named {
        fn set_rect(&mut self, _: Rect, _: &mut LayoutCtx) {}
        fn paint(&self, _: &mut PaintCtx) {}
        fn hit(&self, _: f32, _: f32) -> bool { false }
        fn id(&self) -> Option<WidgetId> { Some(ids::SEARCH_BAR) }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }
    assert_eq!(Named.id(), Some(ids::SEARCH_BAR));
}
```

- [ ] **Step 2: 跑测试确认失败（`WidgetId`、`ids` 未定义）**

Run: `cargo test -p ui widget::tests::widget_id_default_is_none`
Expected: 编译失败。

- [ ] **Step 3: 在 `crates/ui/src/core/widget.rs` 顶部 `use` 之后追加**

```rust
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub struct WidgetId(pub u64);

pub mod ids {
    use super::WidgetId;
    pub const SEARCH_BAR: WidgetId = WidgetId(1);
}
```

- [ ] **Step 4: 在 `Widget` trait 中追加默认方法**

把当前 `pub trait Widget {` 内追加（放在 `is_capturing` 之后）：

```rust
/// 焦点路由用的稳定 ID。需要被键盘焦点定位的 widget 必须 override。
fn id(&self) -> Option<WidgetId> { None }
```

- [ ] **Step 5: 给 `PaintCtx` 加 offset / global_alpha 字段，给 `EventCtx` 加 cursor_hint 字段**

`crates/ui/src/core/widget.rs:46`：

```rust
pub struct PaintCtx<'a> {
    pub list: &'a mut DrawList,
    pub theme: &'a Theme,
    pub dpi: f32,
    pub offset: (f32, f32),
    /// 全局透明度（0.0–1.0）。本次重构仅占位字段，helper / paint_backend 暂未读取。
    /// 默认 1.0；用于后续淡入淡出动画（如 sidebar HoverPeek fade）。
    pub global_alpha: f32,
}

pub struct EventCtx<'a> {
    pub theme: &'a Theme,
    pub dpi: f32,
    /// Widget 在 on_event 处理 MouseMove 时可写入此字段以覆盖默认 cursor。
    /// dispatch 完成后由调用方（events.rs）读取，转为 AppAction::SetCursor。
    /// 容器 dispatch 不读不清，多 widget 写入时后写者覆盖。
    pub cursor_hint: Option<winit::window::CursorIcon>,
}
```

修复测试模块里所有构造 `PaintCtx { ... }` / `EventCtx { ... }` 的地方：
- `PaintCtx { ... }` 追加 `offset: (0.0, 0.0), global_alpha: 1.0,`
- `EventCtx { ... }` 追加 `cursor_hint: None,`

grep 全 workspace：`PaintCtx {` / `EventCtx {`，逐处补字段。

⚠️ 这一步是**跨 crate** 的字面量扫描——`crates/ui` 与 `crates/app` 的测试都构造这些上下文。一次性加完，避免后续美化方案再次触发同款扫描。

- [ ] **Step 6: 加 cursor_hint 默认值的单元测试**

```rust
#[test]
fn event_ctx_cursor_hint_default_is_none() {
    let theme = Theme::dark();
    let ctx = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };
    assert!(ctx.cursor_hint.is_none());
}

#[test]
fn paint_ctx_global_alpha_default_is_one() {
    let theme = Theme::dark();
    let mut dl = DrawList::new();
    let ctx = PaintCtx {
        list: &mut dl, theme: &theme, dpi: 1.0,
        offset: (0.0, 0.0), global_alpha: 1.0,
    };
    assert_eq!(ctx.global_alpha, 1.0);
}
```

- [ ] **Step 7: 跑全 workspace 测试**

Run: `cargo test --workspace`
Expected: 全部通过（字段扫漏会编译失败）。

### Task 1.3 提交阶段 1

- [ ] **Step 1: 验证全 workspace 编译干净**

Run: `cargo build --workspace` — Expected: 无 warning/error。

⚠️ `EventCtx` / `PaintCtx` 字面量遍布 ui + app 两个 crate 的测试，必须全 workspace 编译通过。

- [ ] **Step 2: 提交**

```bash
git add crates/ui/src/core/widget.rs crates/ui/src/core/paint.rs
# 由于字面量扫描波及 crates/app 的测试文件，可能也要 add。grep 之前的字面量补齐。
git commit -m "feat(ui-core): 引入 WidgetId、PaintCtx.offset/global_alpha、EventCtx.cursor_hint 协议"
```

---

## 阶段 2：Dock 容器接管偏移量（paint + dispatch + layout_rect 缓存）

**目标：** Dock 在 layout 时缓存子节点 rect，在 paint/dispatch 时推弹 offset、把鼠标事件转换为相对坐标。Widget 业务代码不动，仍按绝对坐标工作（兼容期）。

**Files:**
- Modify: `crates/ui/src/core/dock.rs`

### Task 2.1 写失败测试：`DockChild.layout_rect` 由 layout 写入

- [ ] **Step 1: 在 `crates/ui/src/core/dock.rs` 的 `tests` 模块追加测试**

```rust
#[test]
fn layout_caches_child_rect_on_dockchild() {
    let mut dock = Dock::new(Box::new(StubWidget::new()));
    dock.children.push(DockChild {
        widget: Box::new(StubWidget::new()),
        side: Side::Top,
        thickness: Box::new(|_, _| 32.0),
        visible: true,
        layout_rect: Rect::ZERO,
    });
    let theme = dummy_theme();
    let mut measure = NoopMeasure;
    let mut ctx = LayoutCtx { measure: &mut measure, theme: &theme, dpi: 1.0 };
    dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut ctx);
    assert_eq!(dock.children[0].layout_rect, Rect::new(0.0, 0.0, 800.0, 32.0));
}

#[test]
fn layout_caches_fill_rect() {
    let mut dock = Dock::new(Box::new(StubWidget::new()));
    dock.children.push(DockChild {
        widget: Box::new(StubWidget::new()),
        side: Side::Top,
        thickness: Box::new(|_, _| 32.0),
        visible: true,
        layout_rect: Rect::ZERO,
    });
    let theme = dummy_theme();
    let mut measure = NoopMeasure;
    let mut ctx = LayoutCtx { measure: &mut measure, theme: &theme, dpi: 1.0 };
    dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut ctx);
    assert_eq!(dock.fill_rect(), Rect::new(0.0, 32.0, 800.0, 568.0));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ui dock::tests::layout_caches_child_rect_on_dockchild`
Expected: 编译失败 — `layout_rect` 字段不存在、`fill_rect()` 方法不存在。

- [ ] **Step 3: 在 `DockChild` 加 `layout_rect`，`Dock` 加 `fill_rect`**

`crates/ui/src/core/dock.rs:19`：

```rust
pub struct DockChild {
    pub widget: Box<dyn Widget>,
    pub side: Side,
    pub thickness: Box<dyn Fn(&Theme, f32) -> f32>,
    pub visible: bool,
    pub layout_rect: Rect,
}
```

`Dock` 结构体内追加私有字段 `fill_rect: Rect`（默认 `Rect::ZERO`）：

```rust
pub struct Dock {
    pub children: Vec<DockChild>,
    pub fill: Box<dyn Widget>,
    fill_rect: Rect,
}
```

并改 `Dock::new`：`fill_rect: Rect::ZERO,`。新增 getter：

```rust
impl Dock {
    pub fn fill_rect(&self) -> Rect { self.fill_rect }
}
```

- [ ] **Step 4: 改 `Dock::layout` 写入 layout_rect 与 fill_rect**

把现有 `layout` 中"算出 child_rect 后立即调 widget.set_rect"改成：先存 `child.layout_rect = child_rect;` 然后照样调 `set_rect`。在 `self.fill.set_rect(remaining, ctx);` 前赋值 `self.fill_rect = remaining;`。

完整新版（替换 `pub fn layout` 函数体即可，diff 较多，参考新代码）：

```rust
pub fn layout(&mut self, screen: Rect, ctx: &mut LayoutCtx) {
    let mut remaining = screen;

    for child in self.children.iter_mut() {
        if !child.visible {
            child.layout_rect = Rect::ZERO;
            continue;
        }
        let t = (child.thickness)(ctx.theme, ctx.dpi);
        if t <= 0.0 {
            child.layout_rect = Rect::ZERO;
            child.widget.set_rect(Rect::ZERO, ctx);
            continue;
        }
        let t_clamped = t.min(match child.side {
            Side::Top | Side::Bottom => remaining.h,
            Side::Left | Side::Right => remaining.w,
        });
        if t_clamped <= 0.0 {
            child.layout_rect = Rect::ZERO;
            child.widget.set_rect(Rect::ZERO, ctx);
            continue;
        }
        let child_rect = match child.side {
            Side::Top => {
                let r = Rect::new(remaining.x, remaining.y, remaining.w, t_clamped);
                remaining = Rect::new(remaining.x, remaining.y + t_clamped,
                    remaining.w, (remaining.h - t_clamped).max(0.0));
                r
            }
            Side::Bottom => {
                let r = Rect::new(remaining.x, remaining.y + remaining.h - t_clamped,
                    remaining.w, t_clamped);
                remaining = Rect::new(remaining.x, remaining.y,
                    remaining.w, (remaining.h - t_clamped).max(0.0));
                r
            }
            Side::Left => {
                let r = Rect::new(remaining.x, remaining.y, t_clamped, remaining.h);
                remaining = Rect::new(remaining.x + t_clamped, remaining.y,
                    (remaining.w - t_clamped).max(0.0), remaining.h);
                r
            }
            Side::Right => {
                let r = Rect::new(remaining.x + remaining.w - t_clamped, remaining.y,
                    t_clamped, remaining.h);
                remaining = Rect::new(remaining.x, remaining.y,
                    (remaining.w - t_clamped).max(0.0), remaining.h);
                r
            }
        };
        child.layout_rect = child_rect;
        child.widget.set_rect(child_rect, ctx);
    }
    self.fill_rect = remaining;
    self.fill.set_rect(remaining, ctx);
}
```

修补现有所有 `DockChild { ... }` 字面量（包括 ui_shell.rs 与 dock 自身测试），加 `layout_rect: Rect::ZERO,`。grep 关键字 `DockChild {`。

- [ ] **Step 5: 跑测试**

Run: `cargo test -p ui dock::tests`
Expected: 新增 2 个测试通过；原有测试也都通过（layout_rect 不影响旧断言）。

### Task 2.2 写失败测试：`Dock::dispatch` 把鼠标事件转换为相对坐标

- [ ] **Step 1: 在 `crates/ui/src/core/dock.rs` 测试模块追加**

```rust
#[test]
fn dispatch_translates_mouse_event_to_child_local_coords() {
    use std::cell::RefCell;
    struct Recorder { rect: Rect, last: RefCell<Option<(f32, f32)>> }
    impl Widget for Recorder {
        fn set_rect(&mut self, rect: Rect, _: &mut LayoutCtx) { self.rect = rect; }
        fn paint(&self, _: &mut PaintCtx) {}
        fn hit(&self, px: f32, py: f32) -> bool {
            // 局部坐标 hit：相对系下应当 0..w / 0..h
            px >= 0.0 && py >= 0.0 && px < self.rect.w && py < self.rect.h
        }
        fn on_event(&mut self, ev: &Event, _: &mut EventCtx) -> Option<WidgetAction> {
            if let Event::MouseDown { px, py, .. } = ev {
                *self.last.borrow_mut() = Some((*px, *py));
            }
            Some(WidgetAction::Consumed)
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }

    let mut dock = Dock::new(Box::new(StubWidget::new()));
    let recorder = Recorder { rect: Rect::ZERO, last: RefCell::new(None) };
    dock.children.push(DockChild {
        widget: Box::new(recorder),
        side: Side::Right,
        thickness: Box::new(|_, _| 16.0),
        visible: true,
        layout_rect: Rect::ZERO,
    });
    let theme = dummy_theme();
    let mut m = NoopMeasure;
    let mut lctx = LayoutCtx { measure: &mut m, theme: &theme, dpi: 1.0 };
    dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut lctx);

    // child layout_rect: x=784, y=0, w=16, h=600
    let mut ectx = EventCtx { theme: &theme, dpi: 1.0 };
    let res = dock.dispatch(
        &Event::MouseDown { px: 790.0, py: 100.0,
            button: crate::core::widget::MouseButton::Left },
        &mut ectx,
    );
    assert_eq!(res, Some(WidgetAction::Consumed));

    // 取出 recorder 验证它收到的相对坐标 = (6, 100)
    let any = dock.children[0].widget.as_any_mut();
    let r = any.downcast_mut::<Recorder>().unwrap();
    assert_eq!(*r.last.borrow(), Some((6.0, 100.0)));
}

#[test]
fn dispatch_capturing_child_receives_negative_local_coords() {
    use std::cell::RefCell;
    struct Cap { rect: Rect, last: RefCell<Option<(f32, f32)>> }
    impl Widget for Cap {
        fn set_rect(&mut self, rect: Rect, _: &mut LayoutCtx) { self.rect = rect; }
        fn paint(&self, _: &mut PaintCtx) {}
        fn hit(&self, _: f32, _: f32) -> bool { false }
        fn is_capturing(&self) -> bool { true }
        fn on_event(&mut self, ev: &Event, _: &mut EventCtx) -> Option<WidgetAction> {
            if let Event::MouseMove { px, py } = ev {
                *self.last.borrow_mut() = Some((*px, *py));
            }
            Some(WidgetAction::Consumed)
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }
    let mut dock = Dock::new(Box::new(StubWidget::new()));
    dock.children.push(DockChild {
        widget: Box::new(Cap { rect: Rect::ZERO, last: RefCell::new(None) }),
        side: Side::Left,
        thickness: Box::new(|_, _| 200.0),
        visible: true,
        layout_rect: Rect::ZERO,
    });
    let theme = dummy_theme();
    let mut m = NoopMeasure;
    let mut lctx = LayoutCtx { measure: &mut m, theme: &theme, dpi: 1.0 };
    dock.layout(Rect::new(0.0, 0.0, 800.0, 600.0), &mut lctx);

    let mut ectx = EventCtx { theme: &theme, dpi: 1.0 };
    // 鼠标在 (250, 50)：sidebar 在 x=0..200，所以局部 px = 50（不越界），但下面再来一个越界值
    let _ = dock.dispatch(
        &Event::MouseMove { px: 250.0, py: 50.0 },
        &mut ectx,
    );
    let any = dock.children[0].widget.as_any_mut();
    let r = any.downcast_mut::<Cap>().unwrap();
    assert_eq!(*r.last.borrow(), Some((250.0, 50.0)),
        "capturing child 应收到相对坐标，且允许 px > w");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ui dock::tests::dispatch_translates_mouse_event_to_child_local_coords`
Expected: 失败 — 当前 dispatch 直接转发原 ev。

- [ ] **Step 3: 改 `Dock::dispatch` 做相对坐标转换**

新版 dispatch（替换函数体）：

```rust
pub fn dispatch(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
    let is_mouse = matches!(ev,
        Event::MouseMove { .. } | Event::MouseDown { .. }
        | Event::MouseUp { .. } | Event::Wheel { .. });

    if is_mouse {
        for child in self.children.iter_mut().rev() {
            if !child.visible { continue; }
            if child.widget.is_capturing() {
                let local = translate_event(ev, child.layout_rect);
                return child.widget.on_event(&local, ctx);
            }
        }
    }

    for child in self.children.iter_mut().rev() {
        if !child.visible { continue; }
        match ev {
            Event::MouseMove { px, py }
            | Event::MouseDown { px, py, .. }
            | Event::Wheel { px, py, .. } => {
                let lx = *px - child.layout_rect.x;
                let ly = *py - child.layout_rect.y;
                if child.widget.hit(lx, ly) {
                    let local = translate_event(ev, child.layout_rect);
                    return child.widget.on_event(&local, ctx);
                }
            }
            Event::MouseUp { .. } => {
                let local = translate_event(ev, child.layout_rect);
                if let Some(action) = child.widget.on_event(&local, ctx) {
                    return Some(action);
                }
            }
            _ => continue,
        }
    }
    // fall-through 给 fill：转换到 fill 局部坐标
    let local = translate_event(ev, self.fill_rect);
    self.fill.on_event(&local, ctx)
}
```

并在文件底部（impl Dock 之外）追加：

```rust
fn translate_event(ev: &Event, rect: Rect) -> Event {
    match ev {
        Event::MouseMove { px, py } =>
            Event::MouseMove { px: px - rect.x, py: py - rect.y },
        Event::MouseDown { px, py, button } =>
            Event::MouseDown { px: px - rect.x, py: py - rect.y, button: *button },
        Event::MouseUp { px, py, button } =>
            Event::MouseUp { px: px - rect.x, py: py - rect.y, button: *button },
        Event::Wheel { dx, dy, px, py } =>
            Event::Wheel { dx: *dx, dy: *dy, px: px - rect.x, py: py - rect.y },
        Event::KeyDown(k) => Event::KeyDown(*k),
    }
}
```

⚠️ 这一步会让**还在用绝对坐标的 widget**的 hit/on_event 全部失效。**先期望旧测试 fail**，把它们标 `#[ignore = "阶段3之后修复"]` 或者直接同步改写——本计划选**同步改写 dock 自身测试**：

把 `dock_layout_top_then_bottom_leaves_correct_fill` 等测试中"hit 用绝对坐标"的断言改为局部坐标。例如：
- `dock.children[0].widget.hit(0.0, 0.0)` 保持（top child 局部 (0,0)）。
- `bottom_widget.hit(0.0, 576.0)` → `bottom_widget.hit(0.0, 0.0)`（bottom child 局部 y=0）。
- `dock.fill.hit(0.0, 32.0)` → 改为通过 `dock.fill_rect()` 断言：`assert_eq!(dock.fill_rect(), Rect::new(0.0, 32.0, 800.0, 544.0));`

`dock_dispatch_routes_to_topmost_hit` 同步改 — child hit 接相对坐标。

- [ ] **Step 4: 跑测试**

Run: `cargo test -p ui dock::tests`
Expected: 全部通过。

### Task 2.3 写失败测试：`Dock::paint` 推弹 offset

- [ ] **Step 1: 测试新增**

```rust
#[test]
fn paint_pushes_offset_per_child() {
    struct LocalDraw;
    impl Widget for LocalDraw {
        fn set_rect(&mut self, _: Rect, _: &mut LayoutCtx) {}
        fn paint(&self, ctx: &mut PaintCtx) {
            // widget 用 (0,0,10,10) 局部坐标画
            ctx.list.fill_with_offset(
                Rect::new(0.0, 0.0, 10.0, 10.0),
                [1.0, 0.0, 0.0, 1.0],
                ctx.offset,
            );
        }
        fn hit(&self, _: f32, _: f32) -> bool { false }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }
    let mut dock = Dock::new(Box::new(StubWidget::new()));
    dock.children.push(DockChild {
        widget: Box::new(LocalDraw),
        side: Side::Top,
        thickness: Box::new(|_, _| 30.0),
        visible: true,
        layout_rect: Rect::ZERO,
    });
    let theme = dummy_theme();
    let mut m = NoopMeasure;
    let mut lctx = LayoutCtx { measure: &mut m, theme: &theme, dpi: 1.0 };
    dock.layout(Rect::new(100.0, 200.0, 800.0, 600.0), &mut lctx);

    let mut dl = DrawList::new();
    let mut pctx = PaintCtx { list: &mut dl, theme: &theme, dpi: 1.0, offset: (0.0, 0.0) };
    dock.paint(&mut pctx);

    // child 的 fill 应该出现在 (100, 200) — 由容器 push 的 offset 转换
    let found = dl.cmds.iter().any(|c| matches!(c,
        DrawCmd::FillRect { rect, color, .. }
        if *color == [1.0, 0.0, 0.0, 1.0] && rect.x == 100.0 && rect.y == 200.0
    ));
    assert!(found, "child fill 应当被偏移到 (100, 200)，实际 cmds={:?}", dl.cmds);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ui dock::tests::paint_pushes_offset_per_child`
Expected: 失败（fill 出现在 (0, 0)）。

- [ ] **Step 3: 改 `Dock::paint` 推弹 offset**

```rust
pub fn paint(&self, ctx: &mut PaintCtx) {
    let saved = ctx.offset;
    ctx.offset = (saved.0 + self.fill_rect.x, saved.1 + self.fill_rect.y);
    self.fill.paint(ctx);
    ctx.offset = saved;
    for child in self.children.iter() {
        if !child.visible { continue; }
        ctx.offset = (saved.0 + child.layout_rect.x, saved.1 + child.layout_rect.y);
        child.widget.paint(ctx);
    }
    ctx.offset = saved;
}
```

- [ ] **Step 4: 跑测试**

Run: `cargo test -p ui dock::tests`
Expected: 全部通过。

### Task 2.4 提交阶段 2

- [ ] **Step 1: 全 workspace 编译（widget 还没迁移，会有部分测试失败/手测异常，但代码必须编译过）**

Run: `cargo build --workspace`
Expected: 编译通过。

⚠️ 此时各 widget 仍按绝对坐标工作，**应用整体跑起来 chrome 渲染会全部错位**——这是预期的。提交里说明"过渡 commit，阶段 3 后恢复"。

⚠️ 如果 cargo test 全 workspace 有失败，可临时把失败的 app 层测试标 `#[ignore = "等待 widget 迁移完成 (阶段3-5)"]`，并在阶段 5 完成时移除。

- [ ] **Step 2: 提交**

```bash
git add crates/ui/src/core/dock.rs crates/app/src/ui_shell.rs
git commit -m "refactor(dock): 缓存 layout_rect、paint/dispatch 推弹 offset (过渡 commit)"
```

---

## 阶段 3：简单 widget 迁移到相对坐标

每个 widget 一个独立 commit。每个 widget 走相同流程：

A. 改 `set_rect`：把 `self.rect = rect` 改为 `self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);`。
B. 改 `paint`：所有 `ctx.list.fill(...)` 等替换成 `ctx.list.fill_with_offset(..., ctx.offset)`；硬编码 `self.rect.x`、`self.rect.y` 全部清掉，只用 `0.0` 和 `self.rect.w`、`self.rect.h`。
C. 改 `hit`：保持 `self.rect.contains(px, py)` 即可（rect 现在是 0..w / 0..h，等价于局部命中检查）。
D. 改 `on_event`：所有用到 `self.rect.x`/`self.rect.y` 的算式去掉那部分（因为传进来的 px/py 已是局部坐标）。
E. 跑该 widget 的单元测试（位于 `tests` 模块），断言用局部坐标。

### Task 3.1 SearchBarWidget 迁移

**Files:**
- Modify: `crates/ui/src/widgets/search_bar.rs`

- [ ] **Step 1: 改 set_rect**

`crates/ui/src/widgets/search_bar.rs:59` 附近，把 `self.rect = rect;` 改为：

```rust
self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
```

- [ ] **Step 2: 改 paint（搜索 `ctx.list.fill`、`ctx.list.text`、`self.rect.x`、`self.rect.y`）**

`paint` 函数内：
- `ctx.list.fill(self.rect, ctx.theme.search_bar_bg);` → `ctx.list.fill_with_offset(self.rect, ctx.theme.search_bar_bg, ctx.offset);`
- 对所有 `self.rect.x + ...`、`self.rect.y + ...` 表达式去掉前缀：例如 `self.rect.x + self.rect.w - pad_right - count_width` 改为 `self.rect.w - pad_right - count_width`。
- `self.rect.y + self.rect.h * 0.5 + font_size * 0.35` 改为 `self.rect.h * 0.5 + font_size * 0.35`。
- 所有 `ctx.list.text(...)` 改为 `ctx.list.text_with_offset(..., ctx.offset)`。

⚠️ 检查清单：grep `self\.rect\.x\|self\.rect\.y` 在 `crates/ui/src/widgets/search_bar.rs` 应该返回 0 行。

- [ ] **Step 3: 改 hit**

`fn hit(&self, px: f32, py: f32) -> bool { self.rect.contains(px, py) }` 保持不变（rect 现在是 (0,0,w,h)，contains 等价于局部判定）。

- [ ] **Step 4: 加 `id()` 实现**

在 `impl Widget for SearchBarWidget {` 内追加：

```rust
fn id(&self) -> Option<ui_core_widget_id_alias> { Some(crate::core::widget::ids::SEARCH_BAR) }
```

实际写法（不要 alias）：

```rust
fn id(&self) -> Option<crate::core::widget::WidgetId> {
    Some(crate::core::widget::ids::SEARCH_BAR)
}
```

- [ ] **Step 5: 改 on_event 中可能的坐标算式**

读 `crates/ui/src/widgets/search_bar.rs` 中所有 `self.rect.x` / `self.rect.y` 的使用并去掉前缀。本 widget 主要用于显示，事件处理通常仅用 px/py 做命中或文本输入，**预期改动量小**。

- [ ] **Step 6: 改单元测试中绝对坐标断言**

把 `crates/ui/src/widgets/search_bar.rs::tests` 内对 fill cmd 坐标的断言由绝对值改为预期偏移后的绝对值；如果测试中没有用 dock 包装直接跑，则保持局部 (0, 0)。具体改法：测试构造 `PaintCtx` 时 `offset: (0.0, 0.0)` → 与原本绝对结果一致即可（因为以前 widget 把绝对 rect 存进 self.rect.x/y，现在被外部 offset 取代）。

- [ ] **Step 7: 跑测试**

Run: `cargo test -p ui search_bar`
Expected: 全部通过。

- [ ] **Step 8: 提交**

```bash
git add crates/ui/src/widgets/search_bar.rs
git commit -m "refactor(search_bar): 迁移到相对坐标 + 实现 WidgetId"
```

### Task 3.2 StatusBarWidget 迁移

**Files:**
- Modify: `crates/ui/src/widgets/status_bar.rs`

- [ ] **Step 1: set_rect 改局部**

`self.rect = rect;` → `self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);`

- [ ] **Step 2: paint 替换 helper + 去掉绝对坐标前缀**

- `ctx.list.fill(self.rect, ctx.theme.status_bar_bg);` → `ctx.list.fill_with_offset(self.rect, ctx.theme.status_bar_bg, ctx.offset);`
- `self.rect.y + self.rect.h * 0.5 + font_size * 0.35` → `self.rect.h * 0.5 + font_size * 0.35`
- 所有 `ctx.list.text(...)` → `ctx.list.text_with_offset(..., ctx.offset)`，并把 `self.rect.x + ...` 改为相对值。

grep 验证 `self\.rect\.x\|self\.rect\.y` 应为 0。

- [ ] **Step 3: 跑测试**

Run: `cargo test -p ui status_bar`
Expected: 通过。

- [ ] **Step 4: 提交**

```bash
git add crates/ui/src/widgets/status_bar.rs
git commit -m "refactor(status_bar): 迁移到相对坐标"
```

### Task 3.3 TabBarWidget 迁移

**Files:**
- Modify: `crates/ui/src/widgets/tab_bar.rs`

- [ ] **Step 1: set_rect 改局部**

同上模式：`self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);`

- [ ] **Step 2: paint 全量替换 helper**

把所有 `ctx.list.fill(...)` / `ctx.list.text(...)` / `ctx.list.clip(...)` / `ctx.list.fill_rounded(...)` 替换成 `_with_offset` 变体；把所有 `self.rect.x + ...`、`self.rect.y + ...` 去掉前缀。

⚠️ TabBar 内部维护了大量 tab 的子 rect（`tabs` / `back_btn_rect` / `forward_btn_rect`）。如果这些 rect 是基于 set_rect 中的绝对 rect 算出的（grep 查），同样改为以局部坐标为基准。如果不是（仅依赖宽高），则只需替换 helper 调用即可。

- [ ] **Step 3: hit + on_event**

把所有 `self.rect.x + ...` / `self.rect.y + ...` 去掉前缀。

- [ ] **Step 4: 跑测试 + 手测**

Run: `cargo test -p ui tab_bar`
Expected: 通过。

手测（如果时间允许）：跑 `cargo run -p app`，开多个 tab，点击切换、关闭、滚动均正常。

- [ ] **Step 5: 提交**

```bash
git add crates/ui/src/widgets/tab_bar.rs
git commit -m "refactor(tab_bar): 迁移到相对坐标"
```

### Task 3.4 ScrollbarWidget 迁移（注意 capture 越界）

**Files:**
- Modify: `crates/ui/src/widgets/scrollbar.rs`

- [ ] **Step 1: set_rect 改局部**

`self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);` 同时 `compute_layout_px` 仍接受 rect，但传进去的也是局部 rect — 阅读 `compute_layout_px` 实现确认它输出的 thumb_rect 也变成局部坐标（不会依赖 rect.x/rect.y 当作绝对屏幕坐标）。

⚠️ 如果 `compute_layout_px` 内部用了 `rect.x`/`rect.y` 当绝对坐标转出 thumb，需要一起改成局部。grep `compute_layout_px` 看一眼。

- [ ] **Step 2: paint 替换 helper**

- `ctx.list.fill(self.rect, track_color);` → `_with_offset` + `ctx.offset`
- `ctx.list.fill(self.layout.thumb_rect, thumb_color);` → 同上

- [ ] **Step 3: on_event 中 rect.x / rect.y 全部去掉**

`crates/ui/src/widgets/scrollbar.rs:141, 152, 153, 172` 的 `self.rect.x + self.rect.w * 0.5` → `self.rect.w * 0.5`，`self.rect.y` 出现的地方 → `0.0`，`drag_start_thumb_y` 已是 thumb 顶（局部），`raw_y.clamp(0.0, track_range)`。

⚠️ Capture 越界规约：拖拽 thumb 时 `py` 可能为负或 > rect.h。当前代码 `raw_y.clamp(self.rect.y, self.rect.y + track_range)` → 改为 `raw_y.clamp(0.0, track_range)` 即可保护边界。

- [ ] **Step 4: hit 不变**

`self.rect.contains(px, py)` 等价于局部判定。

- [ ] **Step 5: 跑测试**

Run: `cargo test -p ui scrollbar`
Expected: 通过。

- [ ] **Step 6: 手测**

跑 `cargo run -p app`，长文档拖拽 scrollbar，光标移出 widget 矩形仍能继续滚动（capture 行为）。

- [ ] **Step 7: 提交**

```bash
git add crates/ui/src/widgets/scrollbar.rs
git commit -m "refactor(scrollbar): 迁移到相对坐标 + capture 越界保护"
```

---

## 阶段 4a：VerticalListWidget 迁移（sidebar 依赖）

**Files:**
- Modify: `crates/ui/src/widgets/list.rs`

### Task 4a.1 set_rect 改局部 + row_rect 重算

- [ ] **Step 1: set_rect**

`self.rect = rect;` → `self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);`

- [ ] **Step 2: 改 row_rect / hit_row（`crates/ui/src/widgets/list.rs:85-99`）**

把 `top = self.rect.y + pad_y + i as f32 * row_h;` 改成 `top = pad_y + i as f32 * row_h;`，相应 `bottom > self.rect.y + self.rect.h` 改为 `bottom > self.rect.h`，`Rect::new(self.rect.x, top, ...)` → `Rect::new(0.0, top, ...)`。

`hit_row` 的 `self.rect.contains(px, py)` 保持不变（rect 0,0,w,h）。

### Task 4a.2 paint 替换 helper

- [ ] **Step 1: 把所有 `ctx.list.fill / text / clip` 替换为 `_with_offset` 变体并传 `ctx.offset`。**

注意 `paint` 中可能有循环画每行，行 rect 已经是局部，直接交给 helper 即可。

### Task 4a.3 跑测试 + 提交

- [ ] **Step 1: 测试**

Run: `cargo test -p ui list`
Expected: 通过。如果原测试用绝对坐标断言行 rect，按"行 y = pad_y + i*row_h"修订断言。

- [ ] **Step 2: 提交**

```bash
git add crates/ui/src/widgets/list.rs
git commit -m "refactor(list): 迁移到相对坐标 + hit_row 重算"
```

---

## 阶段 4b：SidebarLayout + SidebarWidget 主体迁移

**Files:**
- Modify: `crates/ui/src/sidebar.rs`
- Modify: `crates/ui/src/widgets/sidebar.rs`

### Task 4b.1 改 SidebarLayout 内部 rect 为局部坐标

- [ ] **Step 1: 找出所有计算 rect 时引用 `content_top` / 屏幕原点的地方**

`crates/ui/src/sidebar.rs:464` 附近 `bg_rect = Rect::new(0.0, top, w, ...)` —— `top` 来自 `content_top`，`content_top` 当前来自 `rect.y`（widget 绝对坐标）。

新规约：sidebar 的 `set_rect` 传入 layout_rect.h 已经是 sidebar 区域高度，`content_top = title_h_local`（标题栏占的局部高度），不再来自外部。

- [ ] **Step 2: 改 `SidebarInput.content_top` 语义**

`crates/ui/src/widgets/sidebar.rs:250` 当前 `content_top: rect.y,` 改为 `content_top: 0.0,`（内部 layout 自己加 title_h）。
然后在 `compute_layout`（`crates/ui/src/sidebar.rs` 内部）的 `let bg_rect = Rect::new(0.0, top, ...)`，把 `top` 视为 sidebar 局部 y：从 0 开始，加上 title_h。

⚠️ 这一步要先读 `compute_layout` 完整实现（grep `fn compute_layout`），把所有依赖外部绝对坐标的字段（content_top、screen_h 中表示绝对位置的部分）替换为局部值。

- [ ] **Step 3: `menu_btn_rect` 改局部**

`crates/ui/src/sidebar.rs:472` 附近 `menu_btn_rect = Rect::new(menu_x, menu_y, ...)`：menu_x、menu_y 均改为 sidebar 局部坐标（y 从 title_h 顶部偏移即可）。

- [ ] **Step 4: 改 `hit_test_px`（`crates/ui/src/sidebar.rs:672`）**

调用方传的 px/py 现在已经是 sidebar 局部坐标，`layout.menu_btn_rect.contains(px, py)` 自然成立（菜单 rect 也是局部）。**无需改算式**，但要写一个新单元测试验证 hit_test 在 (px=0, py=local_menu_y) 命中。

- [ ] **Step 5: 改 `open_settings_menu` / `dispatch_menu_click` —— menu 改 sidebar 局部坐标**

**现状（`crates/ui/src/sidebar.rs:356-413`）：** menu rect 用 `screen_w / screen_h` 做 clamp，`anchor_x = settings_btn_rect.x`、`anchor_y = settings_btn_rect.bottom()`。`screen_w/h` 是绝对屏幕尺寸，但 `settings_btn_rect` 在阶段 4b.1 之后是 sidebar 局部坐标——**两套坐标系混在同一个表达式里**，必须统一。

**改法：** menu rect 与 item_rects 全部用 sidebar 局部坐标计算。`open_settings_menu` 的 `screen_w / screen_h` 参数语义改成 "sidebar 局部宽/高"（即 `cfg.width` 与 sidebar 局部高度），调用方 `crates/ui/src/widgets/sidebar.rs:227` 把 `self.screen_w/h` 改成 `self.cfg.width` 与 sidebar 局部高度。clamp 也用 sidebar 局部宽。

```rust
pub fn open_settings_menu(&mut self, _current_mode: ViewMode,
    sidebar_local_w: f32, sidebar_local_h: f32)
{
    // ... anchor 来自 settings_btn_rect（已是 sidebar 局部）
    let menu_left = anchor_x.min(sidebar_local_w - menu_w).max(0.0);
    // menu rect 全部 sidebar 局部坐标
}
```

`dispatch_menu_click(px, py)`：调用方 `crates/ui/src/widgets/sidebar.rs:353` 现在传入的 `px/py` 已经是 sidebar 局部（阶段 2 dispatch 减法已生效），命中判断与 menu rect 同坐标系，直接 `menu.hit_test_px(px, py)` 即可。

**screen_size 字段保留 sidebar 局部值**，paint 时这个字段也要在 sidebar 内部画，不再走 overlay 通路。

⚠️ 这一步隐含的限制：menu **不能超出 sidebar 矩形**（被 dock layout_rect 裁掉）。settings menu 高度 4 项 ~120px，宽 200px，远小于 sidebar（min 160px wide），实际不会越界；但视觉上 menu 紧贴 settings 按钮上方/下方，合理。如果未来需要 menu 跨 sidebar 边界，再改成 overlay 通路。

- [ ] **Step 6: 同步修测试**

`crates/ui/src/sidebar.rs::tests::sidebar_settings_menu_*` 共 5 个测试，目前用 `(1200.0, 800.0)` 当 screen 尺寸调用 `open_settings_menu`。改为传 `(cfg.width, sidebar_local_h)`，例如 `(220.0, 800.0)`（screen_h 800 减去 title 0 ≈ 800）。`item_rects[i]` 的坐标断言相应改为 sidebar 局部。

- [ ] **Step 7: 跑相关测试**

Run: `cargo test -p ui sidebar`
Expected: 通过（如果失败，按测试报错把绝对坐标改局部）。

### Task 4b.2 SidebarWidget set_rect / paint / hit / on_event 迁移

- [ ] **Step 1: set_rect 改局部**

`crates/ui/src/widgets/sidebar.rs:240`：
```rust
fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
    self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
    let dpi = ctx.dpi;
    self.cfg.clamp_width(dpi);
    let input = crate::sidebar::SidebarInput {
        tabs: &self.tabs,
        active_index: self.active_index,
        screen_w: self.screen_w,
        screen_h: self.screen_h,
        traffic_light_inset: self.traffic_light_inset,
        content_top: 0.0,  // 局部坐标
    };
    self.state.update_layout(&input, &self.cfg);
    let list_rect = self.state.current_layout()
        .map(|l| l.list_clip)
        .unwrap_or(Rect::ZERO);
    self.list.set_style(make_style_from_theme(ctx.theme));
    let items: Vec<ListItem> = self.tabs.iter().map(|t| ListItem {
        label: t.title.clone(),
        kind: ListItemKind::Normal,
        indicator: if t.is_dirty { ListItemIndicator::Dot } else { ListItemIndicator::None },
    }).collect();
    self.list.set_items(items);
    self.list.set_active(self.active_index);
    self.list.set_rect(list_rect, ctx);  // list_rect 已是 sidebar 局部坐标
}
```

- [ ] **Step 2: paint —— sidebar 自己画框架，再画 list 时推 offset**

`crates/ui/src/widgets/sidebar.rs:273` 的 `paint`：

```rust
fn paint(&self, ctx: &mut PaintCtx) {
    if self.state.current_layout().is_none() { return; }
    self.state.paint(ctx, self.active_index);  // 内部所有 fill/text 改 _with_offset
    // list 的 layout_rect 已是 sidebar 局部坐标，list 内部画 (0,0,w,h)，
    // 但传入 list 的 ctx.offset 还得加上 list_rect 起点。
    let list_rect = self.list.rect();
    let saved = ctx.offset;
    ctx.offset = (saved.0 + list_rect.x, saved.1 + list_rect.y);
    self.list.paint(ctx);
    ctx.offset = saved;
}
```

⚠️ `state.paint` 内部所有 `ctx.list.fill(...)` 必须替换为 `_with_offset`。grep `crates/ui/src/sidebar.rs` 内的 `ctx\.list\.` 改一遍。

- [ ] **Step 3: hit 仍用 layout 内 rect**

```rust
fn hit(&self, px: f32, py: f32) -> bool {
    if let Some(layout) = self.state.current_layout() {
        layout.bg_rect.contains(px, py) || layout.menu_btn_rect.contains(px, py)
    } else { false }
}
```

`bg_rect` 与 `menu_btn_rect` 现在已是 sidebar 局部坐标，调用方传入的 px/py 也是 sidebar 局部，等价。

- [ ] **Step 4: on_event 中的 drag 公式重写**

`crates/ui/src/widgets/sidebar.rs:302` 拖宽：

```rust
if self.dragging {
    let dpi = ctx.dpi;
    // px 现在是 sidebar 局部坐标。drag_start_px 也存局部坐标。
    let mut new_w = self.drag_start_width + (*px - self.drag_start_px);
    let lo = 160.0 * dpi;
    let hi = 400.0 * dpi;
    new_w = new_w.clamp(lo, hi);
    self.cfg.width = new_w;
    return Some(WidgetAction::Sidebar(SidebarAction::ResizeTo(new_w)));
}
```

边缘检测 `crates/ui/src/widgets/sidebar.rs:362-372`：
```rust
let band = 4.0 * dpi;
let edge = self.cfg.width;  // sidebar 宽度（局部 x = edge 即右边缘）
if (px - edge).abs() <= band {
    self.dragging = true;
    self.drag_start_px = px;     // 局部 px
    self.drag_start_width = self.cfg.width;
    return Some(WidgetAction::Sidebar(SidebarAction::StartResize));
}
```

注意去掉 `&& px < self.screen_w`——它只在绝对坐标下有意义。

- [ ] **Step 5: hit_test_px、dispatch_menu_click、hit_row 调用都改为传局部 px/py**

代码里这些调用已经直接转发 ev 中的 px/py，由于阶段 2 已经把 ev 转换为局部，这里**不需要改 caller，只要确保 callee 用局部坐标算 rect**——阶段 4b.1 已经处理。

- [ ] **Step 6: 跑测试**

Run: `cargo test -p ui sidebar`
Expected: 通过。

### Task 4b.3 提交阶段 4b

- [ ] **Step 1: 手测验证**

`cargo run -p app`：
1. sidebar 显示正常，title 不被遮挡。
2. hamburger 按钮可切换 pinned/floating（这是 35381c3、324b97f 的修复点，不能回归）。
3. 拖动右边缘可调宽，光标移出 sidebar 仍能拖动（capture）。
4. 右键 tab 弹出 context menu 在正确位置。

- [ ] **Step 2: 提交**

```bash
git add crates/ui/src/sidebar.rs crates/ui/src/widgets/sidebar.rs
git commit -m "refactor(sidebar): 迁移到相对坐标 + drag 公式重写"
```

---

## 阶段 4c：PopupMenu 迁移

**Files:**
- Modify: `crates/ui/src/widgets/popup_menu.rs`
- Modify: `crates/app/src/ui_shell.rs`（OverlayEntry 包装）

### Task 4c.1 改 OverlayEntry

- [ ] **Step 1: 在 `crates/app/src/ui_shell.rs` 加结构体**

```rust
struct OverlayEntry {
    widget: Box<dyn Widget>,
    layout_rect: Rect,
}
```

- [ ] **Step 2: 把 `overlays: Vec<Box<dyn Widget>>` 改为 `Vec<OverlayEntry>`**

修改 `push_overlay`、`pop_overlay`、`overlays_count`、`paint_chrome`、`paint`、`dispatch` 中的所有访问。

- `push_overlay(&mut self, widget: Box<dyn Widget>, layout_rect: Rect)`：调用方传 rect。
- `pop_overlay` 返回 `Option<Box<dyn Widget>>` 即可（业务上不关心 rect 反馈，dropped）。

- [ ] **Step 3: paint 推 offset**

```rust
pub fn paint(&self, ctx: &mut PaintCtx) {
    self.dock.paint(ctx);
    let saved = ctx.offset;
    for o in &self.overlays {
        ctx.offset = (saved.0 + o.layout_rect.x, saved.1 + o.layout_rect.y);
        o.widget.paint(ctx);
    }
    ctx.offset = saved;
}
```

`paint_chrome` 同理。

- [ ] **Step 4: dispatch 减 offset**

```rust
for entry in self.overlays.iter_mut().rev() {
    let local = match ev {
        Event::MouseMove { px, py } =>
            Event::MouseMove { px: px - entry.layout_rect.x, py: py - entry.layout_rect.y },
        Event::MouseDown { px, py, button } =>
            Event::MouseDown { px: px - entry.layout_rect.x, py: py - entry.layout_rect.y, button: *button },
        Event::MouseUp { px, py, button } =>
            Event::MouseUp { px: px - entry.layout_rect.x, py: py - entry.layout_rect.y, button: *button },
        Event::Wheel { dx, dy, px, py } =>
            Event::Wheel { dx: *dx, dy: *dy, px: px - entry.layout_rect.x, py: py - entry.layout_rect.y },
        Event::KeyDown(k) => Event::KeyDown(*k),
    };
    if let Event::MouseMove { px, py }
       | Event::MouseDown { px, py, .. }
       | Event::MouseUp { px, py, .. }
       | Event::Wheel { px, py, .. } = local
    {
        if !entry.widget.hit(px, py) { continue; }
    }
    if let Some(action) = entry.widget.on_event(&local, ctx) { return Some(action); }
}
self.dock.dispatch(ev, ctx)  // 注意 dock.dispatch 内部本来就处理减法
```

⚠️ 共用一段减法逻辑，可以抽 `translate_event`（与阶段 2 dock 中重复，可以放到 `crates/ui/src/core/dock.rs` 里 `pub fn translate_event` 或 `crates/ui/src/core/widget.rs` 公开）。

### Task 4c.2 PopupMenu set_rect / paint / hit / on_event

- [ ] **Step 1: set_rect 改局部**

`crates/ui/src/widgets/popup_menu.rs:45`：`self.rect = rect;` → `self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);`

- [ ] **Step 2: paint 替换 helper**

替换 fill/text/clip 为 `_with_offset` 版本。

- [ ] **Step 3: hit / on_event**

`hit`、`on_event` 中的 `self.rect.contains(*px, *py)` 不用改（rect 是 0..w / 0..h）；如果有 `self.rect.x + ...` 计算项做相应的去前缀。

### Task 4c.3 调用方传 layout_rect

- [ ] **Step 1: grep `push_overlay(`**

把所有 `push_overlay(box_widget)` 调用点改为 `push_overlay(box_widget, popup_rect)`。popup rect 通常在 `popup_menu.rs` 的构造方法里就算好了——读 `pub fn new(...) -> Self` 看 rect 是怎么进来的，把它从 widget 内部字段提取出来作为参数。

### Task 4c.4 跑测试 + 手测 + 提交

- [ ] **Step 1: 测试**

Run: `cargo test --workspace`
Expected: 全部通过。

- [ ] **Step 2: 手测**

`cargo run -p app`：右键 tab → context menu 出现在光标处；点击菜单项触发动作；点击外部关闭。

- [ ] **Step 3: 提交**

```bash
git add crates/ui/src/widgets/popup_menu.rs crates/app/src/ui_shell.rs
git commit -m "refactor(popup): 迁移到相对坐标 + OverlayEntry 容器"
```

---

## 阶段 5：键盘焦点切换到 WidgetId 路由 + 消除 downcast

**Files:**
- Modify: `crates/app/src/ui_shell.rs`

### Task 5.1 写失败测试：focus 路由通过 id 命中

- [ ] **Step 1: 在 `crates/app/src/ui_shell.rs::tests` 模块追加**

```rust
#[test]
fn forward_key_routes_to_widget_id_search_bar() {
    use ui::core::widget::{ids, KeyCode};
    let theme = test_theme();
    let mut m = NoopMeasure;
    let mut shell = UiShell::new();
    shell.frames_rendered = 1;
    let inputs = ShellInputs {
        tabs_visible: false, tabs_thickness: 0.0,
        search_visible: true, search_thickness: 32.0,
        status_thickness: 0.0,
        sidebar_visible: false, sidebar_thickness: 0.0,
        scrollbar_thickness: 0.0,
        dpi: 1.0,
    };
    shell.update_frame(Screen::new(800.0, 600.0), &theme, &mut m, &inputs);
    // search_visible=true 应当将焦点设到 SEARCH_BAR
    assert_eq!(shell.keyboard_focus, Some(ids::SEARCH_BAR));
    // 转发一个 key 进 SearchBar，应当能被 SearchBarWidget 接住（不再 downcast）
    let _ = shell.forward_key(KeyCode::Escape, &theme, 1.0);
    // 这里只验证 API 存在并不 panic；具体 SearchBarAction 在 SearchBar 单元测试覆盖。
}
```

- [ ] **Step 2: 跑测试确认失败（`keyboard_focus` 类型不匹配）**

Run: `cargo test -p app forward_key_routes_to_widget_id_search_bar`
Expected: 编译失败 — `ids::SEARCH_BAR` 类型是 `WidgetId`，不能赋给 `Option<FocusTarget>`。

### Task 5.2 改 `keyboard_focus` 类型 + 删除 FocusTarget

- [ ] **Step 1: 改字段类型**

`crates/app/src/ui_shell.rs:53`：

```rust
pub keyboard_focus: Option<ui::core::widget::WidgetId>,
```

- [ ] **Step 2: 删除 `FocusTarget` 枚举（39-42 行）**

整个 enum 直接删除。grep `FocusTarget` 应只剩下要修改的地方。

- [ ] **Step 3: 改 `rebuild_dock_children` 中赋值**

`crates/app/src/ui_shell.rs:409` 与 `:411`：
```rust
self.keyboard_focus = Some(ui::core::widget::ids::SEARCH_BAR);
// search_visible=false 分支：
self.keyboard_focus = None;  // editor 不需要 ID（dock.fill 拿键盘事件作为兜底）
```

⚠️ 现状是 `Some(FocusTarget::Editor)`，在 `forward_key` 里只对 `SearchBar` 分支 routes，所以替换成 `None` 是安全的。但要去 grep `keyboard_focus` 所有读取处，确认没有依赖 `FocusTarget::Editor` 的判断（应当只有 `forward_key` 那一处的相等比较）。

### Task 5.3 改 `forward_key` 实现，去掉 downcast

- [ ] **Step 1: 替换函数体**

`crates/app/src/ui_shell.rs:257`：

```rust
pub fn forward_key(
    &mut self,
    key: ui::core::widget::KeyCode,
    theme: &Theme,
    dpi: f32,
) -> Option<ui::widgets::search_bar::SearchBarAction> {
    let focus = self.keyboard_focus?;
    if focus != ui::core::widget::ids::SEARCH_BAR { return None; }
    let ev = Event::KeyDown(key);
    let mut ctx = EventCtx { theme, dpi };
    for child in &mut self.dock.children {
        if child.widget.id() == Some(focus) {
            return child.widget.on_event(&ev, &mut ctx).and_then(|a| match a {
                ui::core::widget::WidgetAction::SearchBar(sa) => Some(sa),
                _ => None,
            });
        }
    }
    None
}
```

⚠️ `self.dock.children` 字段当前是 `pub`（看 `dock.rs:28` `pub children: Vec<DockChild>`），不需要 accessor。

- [ ] **Step 2: 删除 `forward_key` 内对 `SearchBarWidget` 的 downcast**

grep `downcast_mut::<SearchBarWidget` —— `forward_key` 内的那一处现在已被替换；`update_widget_state` 内的那处仍然存在（用于注入 input data，**保留**），不在本任务范围。

### Task 5.4 跑测试 + 手测 + 提交

- [ ] **Step 1: 测试**

Run: `cargo test --workspace`
Expected: 全部通过（包括 5.1 新增测试）。

- [ ] **Step 2: 手测**

`cargo run -p app`：
1. 按 Cmd-F 打开 SearchBar，键盘输入有效。
2. 关闭 SearchBar 后，方向键 / 字符键正常作用于编辑器。
3. 切换 tab、调整 sidebar 宽度等鼠标交互不回归。

- [ ] **Step 3: 提交**

```bash
git add crates/app/src/ui_shell.rs
git commit -m "refactor(focus): 用 WidgetId 替换 FocusTarget downcast 路由"
```

---

## 阶段 6：清理与回归验收

**Files:**
- Modify: 多个测试文件中可能仍用绝对坐标的断言（按需）

### Task 6.1 全工程测试 + 清理 ignored

- [ ] **Step 1: 跑全测试**

Run: `cargo test --workspace`
Expected: 全部通过，0 个 ignored（如果阶段 2 临时 ignore 了某些测试，恢复并修正它们）。

- [ ] **Step 2: 检查 grep 红线**

```bash
grep -rn "self\.rect\.x\|self\.rect\.y" crates/ui/src/widgets/
```
预期：所有命中都是注释或 0 行（widget 业务代码不应再用 `self.rect.x/y`）。例外：`scrollbar.rs` 中 `self.rect.h`、`self.rect.w` 允许保留。

```bash
grep -rn "FocusTarget\|downcast_mut::<SearchBarWidget" crates/app/src/ crates/ui/src/
```
预期：`forward_key` 路径上 0 命中；`update_widget_state` 中的 `downcast_mut::<SearchBarWidget>` 用于注入 input，保留。

### Task 6.2 文档与 changelog

- [ ] **Step 1: 在 `docs/plans_ui_refactor_v2.md` 顶部追加完成时间与 commit 范围**

格式：

```
> 实施完成：YYYY-MM-DD（commit abc123 ~ def456）
```

### Task 6.3 最终回归手测

- [ ] **Step 1: 启动应用走完关键路径**

`cargo run -p app`，逐项确认：
1. ✅ 启动后 sidebar 默认状态正确（pinned/floating 之一）；
2. ✅ Cmd-F 唤起 search bar，输入、Esc 退出；
3. ✅ 拖动 scrollbar，光标移出右侧仍能拖；
4. ✅ 拖动 sidebar 边缘调宽；
5. ✅ 右键 tab 出现 context menu；点击菜单项有效；
6. ✅ Retina 屏（如果可用）下没有 X 遮挡（35381c3 不回归）；
7. ✅ 高亮、光标 y 偏移正确（d757a2e 不回归）。

- [ ] **Step 2: 任何回归 → 在新 commit 修复，不要 amend 阶段提交**

- [ ] **Step 3: 提交（仅文档变更）**

```bash
git add docs/plans_ui_refactor_v2.md
git commit -m "docs: 标记 ui 相对坐标重构完成"
```

---

