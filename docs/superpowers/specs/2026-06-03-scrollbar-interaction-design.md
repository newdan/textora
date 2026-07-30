# Scrollbar 交互模块设计

> 日期：2026-06-03  
> 状态：待审核

## 1. 动机

当前 `app.rs:1401` 的 `scrollbar_vertices()` 只渲染滚动条视觉（6px 轨道 + 滑块），无任何鼠标交互。用户无法点击/拖拽滚动条，只能靠鼠标滚轮和键盘翻页。

## 2. 目标

- 新增 `crates/app/src/scrollbar.rs` 模块，独立管理滚动条渲染与交互
- hover 时变宽（6px → 12px，× scale_factor），可点可拖
- 变宽不影响文字排版（始终 overlay）
- 修复现有 wrap 场景下滚动条比例计算不准确的问题（目前用 `line_count()` 而非 `WrapIndex::total_display_rows()`）

## 3. 设计

### 3.1 模块接口

```rust
// scrollbar.rs — 公开类型

pub(crate) struct ScrollbarState {
    pub hovered: bool,          // 鼠标在滚动条区域
    pub dragging: bool,         // 正在拖拽滑块
    drag_start_y_ndc: f64,      // 拖拽起始 Y (NDC)
    drag_start_scroll: f64,     // 拖拽起始 scroll_top
}

pub(crate) struct ScrollbarLayout {
    bar_left: f32,
    bar_right: f32,             // 默认宽度右边界
    bar_right_wide: f32,        // hover 变宽右边界
    area_top: f32,              // 文本区域顶部 (NDC)
    area_bottom: f32,           // 文本区域底部 (NDC)
    thumb_top: f32,             // 滑块顶部 (NDC)
    thumb_bottom: f32,          // 滑块底部 (NDC)
    thumb_height_ndc: f32,
    max_scroll: f32,            // total_display_rows - visible_rows
}

pub(crate) enum ScrollbarHit { TrackAbove, TrackBelow, Thumb, None }

pub(crate) enum ScrollbarAction { ScrollTo(f64), PageUp, PageDown, StartDrag, None }
```

### 3.2 公开函数

| 函数 | 签名 | 职责 |
|------|------|------|
| `new()` | `ScrollbarState` | 初始化 |
| `compute_layout()` | `(screen_w, screen_h, tbh, status_h, scale, visible_rows, total_display, scroll_top)` → `ScrollbarLayout` | 计算轨道和滑块几何 |
| `hit_test()` | `(ndc_x, ndc_y, &layout)` → `ScrollbarHit` | NDC 坐标命中判定 |
| `generate_vertices()` | `(&layout, &state, theme)` → `Vec<GlyphVertex>` | 生成顶点 |
| `handle_mouse_move()` | `(px, py, screen_w, screen_h, &layout, &mut state)` → `(needs_redraw, is_over_scrollbar)` | hover 检测 |
| `handle_mouse_down()` | `(ndc_x, ndc_y, &layout, scroll_top, &mut state)` → `ScrollbarAction` | 按下处理 |
| `handle_drag()` | `(ndc_y, &layout, &state)` → `Option<f64>` | 拖拽中返回新 scroll_top |
| `handle_mouse_up()` | `(&mut state)` → `bool` | 结束拖拽 |

### 3.3 DPI 适配

所有像素值乘以 `scale_factor`：

```
默认宽度 = 6.0 × scale_factor  px
hover宽度 = 12.0 × scale_factor px
右边距   = 4.0 × scale_factor  px
滑块最小高度 = 8.0 × scale_factor px
```

### 3.4 交互规则

| 操作 | 行为 |
|------|------|
| 鼠标进入滚动条区域 | `hovered=true`，变宽到 12×scale px |
| 鼠标离开 | `hovered=false`，恢复 6×scale px |
| 点击轨道上方空白 | `PageUp`（scroll_top -= visible_rows） |
| 点击轨道下方空白 | `PageDown`（scroll_top += visible_rows） |
| 点击滑块 | `StartDrag` |
| 拖拽滑块 | 按比例实时更新 scroll_top |
| 释放鼠标 | 结束拖拽，clamp scroll_top |
| 空/小文件（total ≤ visible） | 不响应交互，不渲染滑块 |

### 3.5 坐标约定

- 所有几何计算在 **NDC** 空间：`x ∈ [-1, 1]`, `y ∈ [-1, 1]`（y=1 为顶）
- 物理像素 → NDC：`ndc_x = px / screen_w * 2.0 - 1.0`, `ndc_y = 1.0 - py / screen_h * 2.0`
- 滚动条始终 overlay，`bar_left` 保持不变，变宽时 `bar_right` 向右扩展

## 4. app.rs 改动

### 4.1 删除

- `scrollbar_vertices()` 方法（约 40 行）
- 相关 `use` 导入调整

### 4.2 新增字段

```rust
scrollbar: scrollbar::ScrollbarState,
```

### 4.3 事件处理改动

**CursorMoved：** 文本 hit-test 之前先调 `scrollbar::handle_mouse_move()`。命中时设置 cursor 为箭头并跳过文本 hit-test。

**MouseInput(Left, pressed)：** 文本 click 之前先调 `scrollbar::handle_mouse_down()`。根据返回的 `ScrollbarAction` 更新 viewport。

**MouseInput(Left, released)：** 调 `scrollbar::handle_mouse_up()`。

**CursorMoved(拖拽中)：** 如果 `state.dragging`，调 `handle_drag()` 更新 scroll_top。

**WindowEvent::Resized：** 无需特殊处理——scrollbar 每帧通过 `compute_layout()` 与渲染同步更新。

### 4.4 渲染改动

```rust
// 替换：self.scrollbar_vertices(screen_w, screen_h, tbh)
vertices.extend(scrollbar::generate_vertices(&layout, &self.scrollbar, &self.current_theme));
```

## 5. Bug 修复：wrap 场景比例不准

### 问题

当前 `scrollbar_vertices()` 用 `dv.line_count()`（文档行数）算滚动比例，但 `scroll_top` 是 **DisplayRow** 空间的坐标。如果文件有 soft-wrapped 长行，文档行数 ≠ DisplayRow 数，导致滑块大小和位置不准确。

### 修复

`compute_layout()` 接受 `total_display: usize` 参数（来自 `wrap_index.total_display_rows()`），用 DisplayRow 数量计算比例：

```
thumb_ratio = visible_rows / total_display
scroll_ratio = scroll_top / (total_display - visible_rows)
```

## 6. 边界 Case

| 场景 | 行为 |
|------|------|
| 空文件（total=0） | 不渲染滚动条，layout 函数返回 None/空 |
| 小文件（total ≤ visible） | 滚动条存在但不显示滑块，不响应交互 |
| 大文件滑块 < 最小高度 | 强制 min_height = 8×scale px |
| 点在滑块与轨道交界处 | 优先判为 Thumb |
| 拖拽到轨道外部 | 继续跟随鼠标 Y，scroll_top 被 clamp |
| 拖拽中鼠标离开窗口 | 继续拖拽直到 MouseInput(released) |
| 拖拽中窗口 resize | 下一帧 layout 更新，拖拽用新 layout 重算 |
| 切换 tab | 重新 layout，重置 hover/drag 状态 |
| scale_factor 变化 | 下一帧 layout 自动跟随新 scale |
| 快速连点轨道 | 每次独立处理，clamp 保证不越界 |
| 右键点击滚动条 | 忽略，由系统处理 |

## 7. 测试计划

### 7.1 单元测试（scrollbar.rs `#[cfg(test)]`）

**Layout 测试：**
- `layout_tiny_file` — total=5, visible=10 → 滑块占满
- `layout_normal` — total=100, visible=10 → 滑块 10%
- `layout_large_file` — total=100000 → 滑块 ≥ min_height
- `layout_scroll_mid` — scroll_top=50 → 滑块居中
- `layout_scroll_top/bottom` — 边界值
- `layout_dpi_scaling` — scale=1.5 → 宽度 ×1.5

**Hit-test 测试：**
- `hit_above_thumb` / `hit_below_thumb` / `hit_thumb` / `hit_outside`
- `hit_at_thumb_boundary` — 边界优先判为 Thumb

**Drag 测试：**
- `drag_to_top` → scroll_top=0
- `drag_to_bottom` → scroll_top=max
- `drag_proportional` → 拖到 25% → scroll_top=0.25×max

**Vertices 测试：**
- `vertices_default_narrow` — 非 hover 用默认宽度
- `vertices_hover_wide` — hover 用宽宽度

### 7.2 集成测试（现有测试框架）

- 鼠标移入移出滚动条区域，cursor icon 正确切换
- 点击轨道翻页后 scroll_top 和 cursor 可见性正确
- 拖拽后 scroll_top 在有效范围内

## 8. 不变式

- 滚动条始终 overlay，不影响文字排版（bar_left 固定）
- 所有比例计算基于 `WrapIndex::total_display_rows()`，非 `line_count()`
- NDC 坐标转换公式与 tab_bar、status_bar 等模块一致
- 拖拽结束后 scroll_top 必定 clamp
