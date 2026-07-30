# SplitterWidget 设计方案

## 1. 概述

将 Sidebar 右侧边线（弧线、竖直线、水平线、填充）和 resize 拖拽逻辑从 Sidebar 中抽离，封装为独立的 `SplitterWidget`。

### 目标

- Sidebar 不再负责边框绘制
- Splitter 作为独立 Dock child，与 Sidebar 并列，**始终存在**（不管 Sidebar 是否可见）
- 弧线从 Splitter 左边界出发，向右弯进 Splitter 右边界
- 整个 Splitter 区域都是抓取区

---

## 2. Dock 布局

```
┌──────────┬──────┬──────────────────────┐
│ Sidebar  │Split │      内容区           │
│ (Left)   │(Left)│    (fill_rect)       │
│          │12dp  │                      │
└──────────┴──────┴──────────────────────┘
```

- Sidebar: `Side::Left`，宽度 `cfg.width`
- Splitter: `Side::Left`，宽度 `grab = 12dp`
- 顺序：Sidebar 在前，Splitter 在后（紧贴 Sidebar 右侧）
- **Splitter 始终在 Dock 中**，不管 Sidebar 可见性如何

---

## 3. Splitter 本地坐标系

```
 x=0                     x=r=8dp        
  │                        │      
  │ (边界, sidebar 右缘)    │     
  │                        │   
  │      ┌─ 弧线区 ─┐      │
  │      │ content  │      │      
  │      │ _bg fill │      │  
  │      │ + border │      │    
  │      └──────────┘      │      
  │                        │      
```

- `x=0`：sidebar 右边界 = 内容区左边界
- `x=0 .. x=r`：弧线 + 竖直线 + content_bg 填充区域
- `x=r .. x=grab`：剩余抓取区
- 整个 `x=0 .. x=grab` 都响应 hit（用于 resize/peek）
- **y=0 跳过 header 区域**，从 header_h 开始绘制和响应交互

---

## 4. 弧线几何

### 4.1 常量

```
r       = 8.0  * dpi     // 弧线半径
bw      = 1.0  * dpi     // 边框线宽
grab    = 12.0 * dpi     // Splitter 总宽度
status_h = status_bar_height_for_dpi(dpi)       // 28dp * dpi
```

> `title_h` 不需要：Splitter 本地坐标 y=0 就是内容区顶部（Dock 布局已扣除 TitleBar/TabBar）。

### 4.2 顶部弧线 (TL corner mask)

```
rect: (x=0, y=0, w=r, h=2r+1)
mask: 0b0001 (TL only)

   x=0 (边界)              x=r
    │                       │
    ├─────────────────╮     │  y=0                ← 弧终点 (r, 0)
    │                ╱      │
    │               ╱       │
    │    content_bg ╱       │
    │   (fill 内部)╱        │
    │             ╱         │
    │            ╱          │
    │╭          ╱           │  y=r                ← 弧起点 (0, r)
    ├╰─────────╯────────────┤
    ├───────────────────────┤  y=2r+1             ← rect 底边
```

- **圆心**：`(r, r)`（TL 角内侧）
- **圆弧**：1/4 圆，从左边 `(0, r)` 弯到顶边 `(r, 0)`
- **fill**: `fill_rounded_masked(rect, content_bg, r, 0b0001)` — TL 角圆角，其余三角直角
- **stroke**: `stroke_rounded_masked(rect, border, r, bw, 0b0001)`

### 4.3 底部弧线 (BL corner mask)

```
rect: (x=0, y=h-2r, w=r, h=2r+1)
mask: 0b0100 (BL only)

   x=0 (边界)              x=r
    │                       │
    ├───────────────────────┤  y=h-2r             ← rect 顶边
    │          ╲            │
    │           ╲           │
    │  content_bg╲          │
    │ (fill 内部) ╲         │
    │              ╲        │
    │               ╲       │
    │╭               ╲      │  y=h-r              ← 弧起点 (0, h-r)
    ├╰────────────────╲─────┤
    ├───────────────────╮   │  y=h                ← 弧终点 (r, h)
```

- **圆心**：`(r, h-r)`（BL 角内侧）
- **圆弧**：1/4 圆，从左边 `(0, h-r)` 弯到底边 `(r, h)`
- **fill**: `fill_rounded_masked(rect, content_bg, r, 0b0100)`
- **stroke**: `stroke_rounded_masked(rect, border, r, bw, 0b0100)`

### 4.4 弧线方向总览

```
  Sidebar 侧                Splitter 侧 (x >= 0)

      sidebar_bg           x=0               x=r
        │                   │                  │
  ══════╪═══════════════════╪══╗               │   top 弧 (TL mask)：(0,r)→(r,0)
        │                   │  ║               │
        │                   │  ║  content_bg   │
        │                   │  ║               │
        │                   │  ║  | 竖直线     │
        │                   │  ║  | (x=0,bw)   │
        │                   │  ║               │
  ══════╪═══════════════════╪══╝               │   bottom 弧 (BL mask)：(0,h-r)→(r,h)
        │                   │                  │
```

---

## 5. 绘制步骤

按绘制顺序（底部先画，顶部后画覆盖）：

```
// 0. Splitter 背景：透明（无需绘制）

// 1. 底部弧线 fill + stroke
fill_rounded_masked( Rect(0, h-2r, r, 2r+1), content_bg, r, 0b0100 )
stroke_rounded_masked( Rect(0, h-2r, r, 2r+1), border, r, bw, 0b0100 )

// 2. 底部状态栏填充（连接底弧和状态栏）
fill( Rect(0, h - status_h, r, status_h), sidebar_bg )

// 3. 中间 content_bg 填充条
let mid_top = r;               // 顶弧下方
let mid_bot = h - r;           // 底弧上方
if mid_bot > mid_top:
    fill( Rect(0, mid_top, r, mid_bot - mid_top), content_bg )

// 4. 竖直线（x=0 处，左边线）
if mid_bot > mid_top:
    fill( Rect(0, mid_top, bw, mid_bot - mid_top), border )

// 5. 顶部弧线 fill + stroke
fill_rounded_masked( Rect(0, 0, r, 2r+1), content_bg, r, 0b0001 )
stroke_rounded_masked( Rect(0, 0, r, 2r+1), border, r, bw, 0b0001 )

// 6. 顶部水平线（对齐 titlebar 分割线）
fill( Rect(0, 0, r, bw), border )
```

### 裁剪策略

Splitter 作为独立 Dock child 插入在 Sidebar 之后，**绘制顺序在 Sidebar 之上**。Splitter 的 content_bg 填充自然覆盖 Sidebar 右边缘的内容溢出，无需额外弧线裁剪机制。

---

## 6. 两种模式

### Sidebar 隐藏 (Visibility::Hidden)

| 项 | 行为 |
|---|---|
| 边框绘制 | 不绘制（`paint` 直接 return） |
| 交互 | hover 触发 peek（Sidebar 滑出） |
| 宽度 | 12dp（与 Pinned 相同，不变） |
| 光标 | 默认 cursor |

**实现**：`paint` 检查 sidebar visibility，Hidden 时跳过。`on_event` 中 hover → `SplitterAction::Peek`。

### Sidebar 固定 (Visibility::Pinned)

| 项 | 行为 |
|---|---|
| 边框绘制 | 完整绘制 |
| 交互 | hover → `ew-resize` 光标；拖拽 → resize |
| 宽度 | 12dp |
| 光标 | `ew-resize` |

**实现**：`paint` 正常绘制。`on_event` drag → `SplitterAction::Resize { delta }`。

---

## 7. SplitterAction / EventResult

复用现有 `EventResult`，新增 `Peek` 变体：

```rust
pub enum EventResult {
    // ... 现有变体 ...
    Peek,                      // hover 触发 sidebar peek
    ResizeTo { width: f32 },   // 拖拽 resize（已有）
}
```

- Splitter 的 `on_event` 返回 `EventResult`
- Dock 已有机制将 child 的 EventResult 向上传递
- UiShell 的 match 分支处理 `Peek`

---

## 8. 状态更新机制

Splitter 构造时只接收不变参数（`dpi`、`grab_width`）。可变状态每帧通过 `set_input` 方法更新：

```rust
pub struct SplitterInput {
    pub visibility: Visibility,
    pub header_h: f32,
    pub status_bar_height: f32,
}
```

在 `UiShell::update_widget_state` 中每帧调用 `splitter.set_input(input)`，与 SidebarWidget 接收 tabs 数据的模式一致。

---

## 9. 交互行为

### 拖拽 resize

- Pinned 模式下，整个 Splitter 区域（12dp）响应拖拽
- 拖拽时 `is_capturing` 返回 true，Dock 将所有鼠标事件路由给 Splitter
- Sidebar 宽度 clamp 范围：`[160*dpi, 400*dpi]`
- 鼠标释放结束拖拽

### Hover peek

- Hidden 模式下，鼠标进入 Splitter 区域即触发 peek
- Sidebar 从左侧滑出，鼠标移开后自动收回（有动画）
- peek 期间可在 Sidebar 上操作（点击列表项等）
- **peek 动画方案单独设计**（当前无动画，需实现滑入/滑出效果）

---

## 10. 涉及文件

| 操作 | 文件 | 说明 |
|---|---|---|
| **新建** | `crates/ui/src/widgets/splitter.rs` | SplitterWidget |
| 修改 | `crates/ui/src/widgets/mod.rs` | 导出 splitter 模块 |
| 修改 | `crates/ui/src/widgets/sidebar.rs` | 删边框绘制 + edge_resize_rect + drag 字段 |
| 修改 | `crates/ui/src/sidebar.rs` | SidebarState::paint 删 ~50 行边框代码；SidebarLayout 删 `edge_resize_rect` |
| 修改 | `crates/app/src/ui_shell.rs` | `rebuild_dock_children` 插入 Splitter；`update_widget_state` 加 set_input 调用 |
| 修改 | `crates/ui/src/core/paint.rs` | DrawCmd 加 `mask` 字段；新增 `fill_rounded_masked` / `stroke_rounded_masked` |
| 修改 | `crates/app/src/paint_backend.rs` | `push_fill` / `push_stroke` 按 mask 跳过对应 corner |

### 不修改

- `crates/ui/src/core/dock.rs` — 不碰
- `crates/ui/src/title_bar.rs` — 不碰
- 列表 margin/clip — 不碰

---

## 11. 从 Sidebar 中删除的旧代码

### SidebarLayout

```diff
- pub edge_resize_rect: Rect,
```

### SidebarState::paint 中删除（~50 行）

```rust
// Right border: ... 整个边框绘制块
// 包括 radius, border_w, content_bg, bx, by, bh, bb 等变量
// 顶部弧线 fill/stroke
// 底部弧线 fill/stroke
// 中间 fill strip
// 竖直线
// 顶部水平线
// 底部状态栏矩形
```

### SidebarState::update_layout 中删除

```diff
- let edge_resize_rect = Rect::new(w - edge_w, top + header_h, edge_w * 2.0, sh - header_h);
- edge_resize_rect,
```

### SidebarWidget 中删除

```diff
- dragging: bool,
- drag_start_px: f32,
- drag_start_width: f32,
```
以及 `on_event` 中的 drag/resize 逻辑。

---

## 12. 边界情况

| 场景 | 处理 |
|---|---|
| 窗口极窄，Splitter + Sidebar 超出屏幕 | Sidebar 宽度已被 clamp 到 160dpi，Splitter 固定 12dp，内容区可压缩到 0 |
| 拖拽中窗口 resize | 拖拽继续，宽度 clamp 保证不越界 |
| Hidden 模式下 peek 动画中鼠标移回 | 动画方案单独设计时处理 |
| TabBar 不可见时 Splitter 的 y 范围 | Splitter 的 rect 由 Dock 布局决定，自动适配 |

---

# 开发计划

## 阶段 1：Paint 管线 — masked primitive

**目标**：在 DrawCmd 中支持 per-corner mask，不改变现有 API 行为。

### 1.1 修改 DrawCmd 结构

**文件**：`crates/ui/src/core/paint.rs`

- `FillRect` 加 `mask: u8` 字段，默认值 `0xF`（四角全圆角）
- `StrokeRect` 加 `mask: u8` 字段，默认值 `0xF`
- 现有 `fill`、`fill_rounded`、`stroke`、`stroke_rounded` 方法传入 `mask: 0xF`，行为不变

### 1.2 新增 helper 方法

**文件**：`crates/ui/src/core/paint.rs`

```rust
fn fill_rounded_masked(&mut self, rect: Rect, color: Color, radius: f32, mask: u8)
fn stroke_rounded_masked(&mut self, rect: Rect, color: Color, radius: f32, line_width: f32, mask: u8)
```

### 1.3 修改 paint_backend 支持 mask

**文件**：`crates/app/src/paint_backend.rs`

- `push_fill`：接收 `mask` 参数。遍历 4 个角时，检查 `mask & corner_bit`：
  - 为 1：生成弧线顶点（现有逻辑）
  - 为 0：生成直角顶点（radius 视为 0）
- `push_stroke`：同理，mask=0 的角用直角连接而非弧线

**验证**：编译通过，现有 UI 无视觉变化（默认 mask=0xF）。

---

## 阶段 2：SplitterWidget 基础框架

**目标**：创建 SplitterWidget，实现结构和绘制逻辑。

### 2.1 创建 splitter.rs

**文件**：`crates/ui/src/widgets/splitter.rs`

```rust
pub struct SplitterWidget {
    rect: Rect,
    dpi: f32,
    grab: f32,           // 12dp
    radius: f32,         // 8dp
    border_w: f32,       // 1dp
    // 通过 set_input 更新的状态
    visibility: Visibility,
    header_h: f32,
    status_bar_height: f32,
    // drag 状态
    dragging: bool,
    drag_start_px: f32,
    drag_start_width: f32,
}
```

方法：
- `new(dpi: f32) -> Self`
- `set_input(&mut self, input: SplitterInput)`
- `set_rect(&mut self, rect: Rect)` — 设置 Dock 分配的 rect
- `paint(&self, ctx: &mut PaintCtx)` — 按第 5 节绘制步骤执行
- `on_event(&mut self, event: &Event) -> EventResult` — hover/drag 处理
- `hit(&self, px: f32, py: f32) -> bool` — 整个 rect 区域
- `is_capturing(&self) -> bool` — dragging 时返回 true

### 2.2 导出模块

**文件**：`crates/ui/src/widgets/mod.rs`

添加 `pub mod splitter;`

**验证**：编译通过。

---

## 阶段 3：EventResult 扩展

**目标**：支持 Peek 事件传递。

### 3.1 添加 EventResult::Peek

**文件**：`crates/ui/src/core/event.rs`（或 EventResult 所在文件）

```rust
pub enum EventResult {
    // ... 现有变体 ...
    Peek,
}
```

### 3.2 UiShell 处理 Peek

**文件**：`crates/app/src/ui_shell.rs`

在处理 Dock child EventResult 的 match 中加 `Peek` 分支，触发 sidebar peek 状态切换。

**验证**：编译通过。

---

## 阶段 4：Dock 集成

**目标**：将 Splitter 插入 Dock 布局，连接 set_input。

### 4.1 rebuild_dock_children 插入 Splitter

**文件**：`crates/app/src/ui_shell.rs`

在 SidebarWidget 之后插入 SplitterWidget（`Side::Left`，固定宽度 12dp）。Splitter 始终插入，不管 sidebar_visible。

### 4.2 update_widget_state 更新 Splitter

**文件**：`crates/app/src/ui_shell.rs`

通过 downcast 获取 SplitterWidget，调用 `set_input` 传入 visibility、header_h、status_bar_height。

**验证**：运行程序，Splitter 区域可见（即使 Sidebar 隐藏也能看到 12dp 的 hit 区域）。

---

## 阶段 5：Sidebar 清理

**目标**：从 Sidebar 中移除已迁移到 Splitter 的代码。

### 5.1 删除 SidebarLayout::edge_resize_rect

**文件**：`crates/ui/src/sidebar.rs`

### 5.2 删除 SidebarState::paint 中的边框绘制代码

**文件**：`crates/ui/src/sidebar.rs`

删除 ~50 行边框绘制代码（顶部弧线、底部弧线、中间 fill strip、竖直线、顶部水平线、底部状态栏矩形）。

### 5.3 删除 SidebarWidget 的 drag 字段和逻辑

**文件**：`crates/ui/src/widgets/sidebar.rs`

删除 `dragging`、`drag_start_px`、`drag_start_width` 字段。删除 `on_event` 中的 drag/resize 处理。

### 5.4 删除 SidebarState 的 drag 相关方法

**文件**：`crates/ui/src/sidebar.rs`

删除 `on_drag_start`、`on_drag`、`on_drag_end` 方法。

**验证**：
- 编译通过
- Sidebar 显示正常（无边框，边框由 Splitter 绘制）
- 拖拽 resize 正常工作（由 Splitter 处理）
- Hidden 模式下 hover peek 正常触发

---

## 阶段 6：Peek 动画（单独设计）

**目标**：实现 Sidebar peek 的滑入/滑出动画。

> 当前 peek 无动画，直接切换状态。本阶段需要单独设计方案后实施。
