# Viewport & 虚拟行架构对比分析

> 对比对象：Zed Editor (`/Users/dan/proj/llmws/zed`) vs Edit+ (`/Users/dan/proj/llmws/edit+`)
> 分析日期：2026-05-31
> 目的：分析 Edit+ 视口设计的功能与性能问题，对比 Zed 的设计方案，定位视图刷新时自动滚动的根因。

---

## 1. 核心架构差异：坐标空间

### Zed：单一坐标空间（DisplayRow）

Zed 的整个渲染管线只使用一个坐标空间 —— **DisplayRow**。

```
Buffer (raw bytes)
  → InlayMap   (插入 inlay hints)
  → FoldMap    (折叠)
  → TabMap     (tab → spaces)
  → WrapMap    (soft wrap)
  → BlockMap   (diagnostic blocks)
  → DisplayMap (highlights)
  ↓
DisplayRow / DisplayPoint —— 渲染管线的唯一坐标
```

**关键设计：**
- `scroll_position.y: f64` 是 **DisplayRow** 单位（支持小数，实现亚行像素滚动）
- `start_row = DisplayRow((scroll_position.y + clipped_top_in_lines).floor() as u32)`
- `end_row = DisplayRow((scroll_position.y + clipped_top_in_lines + visible_height_in_lines).ceil() as u32)`
- `layout_lines(rows: Range<DisplayRow>)` 接收的也是 DisplayRow

**整个管线不存在"文档行"概念** —— fold、wrap、inlay 全部在 DisplayMap 层统一处理。渲染层不需要知道一行是被 wrap 的、被 fold 的、还是被 inlay 展开的。

```rust
// crates/editor/src/element.rs:8059-8065
let start_row = cmp::min(
    DisplayRow((scroll_position.y + clipped_top_in_lines).floor() as u32),
    max_row,
);
let end_row = cmp::min(
    (scroll_position.y + clipped_top_in_lines + visible_height_in_lines).ceil() as u32,
    max_row.next_row().0,
);
```

### Edit+：双坐标空间（doc_line vs visual_row）

Edit+ 同时使用两个坐标空间：

| 字段 | 含义 | 来源 |
|------|------|------|
| `scroll_y: f64` | 视觉行位置（wrap 后） | 用户滚动 / autoscroll |
| `scroll_line: usize` | 文档行号（wrap 前） | 由 `update_scroll_line()` 从 `scroll_y` 推导 |
| `visible_range()` | 文档行范围 | `scroll_line..(scroll_line + visible_rows)` |

**渲染管线在两个空间之间反复切换：**
1. `shape_visible_lines` 遍历 `visible_range()` 的**文档行**
2. 对每条文档行做 word wrap 产生**虚拟行**
3. 用 `skip_visual` 跳过首行的前 N 条虚拟行
4. 用 `visual_line_counter` 跟踪已渲染的**虚拟行**数
5. autoscroll 用 `cursor_abs_vl`（**虚拟行**）判断是否需要滚动
6. NOT-IN-VLI 路径退回**文档行**级别操作

**这是所有问题的根源。** 两个坐标空间的转换是有损的、脆弱的。

---

## 2. 滚动锚定机制对比

### Zed：ScrollAnchor = 文档锚点 + 亚行偏移

```rust
// crates/editor/src/scroll.rs:38-42
pub struct ScrollAnchor {
    pub offset: gpui::Point<ScrollOffset>,  // 亚行像素偏移
    pub anchor: Anchor,                      // 文档中的稳定锚点
}
```

**核心特性：**
- 锚点指向 buffer 中的一个**稳定位置**（不因 buffer 编辑而移动，使用 `Anchor` 类型）
- `offset.y` 是亚行偏移（0.0..1.0 之间的分数部分表示像素级偏移）
- `scroll_position()` 动态计算：`anchor.to_display_point(snapshot).row().as_f64() + offset.y`
- 当 buffer 内容改变时，`Anchor` 自动跟踪文本移动，**scroll 位置不会漂移**

```rust
// crates/editor/src/scroll.rs:51-60
pub fn scroll_position(&self, snapshot: &DisplaySnapshot) -> gpui::Point<ScrollOffset> {
    self.offset.apply_along(Axis::Vertical, |offset| {
        if self.anchor == Anchor::Min {
            0.
        } else {
            let scroll_top = self.anchor.to_display_point(snapshot).row().as_f64();
            (offset + scroll_top).max(0.)
        }
    })
}
```

### Edit+：scroll_y 数值 + scroll_line 缓存

```rust
// crates/app/src/viewport.rs:8-17
pub struct Viewport {
    pub scroll_line: usize,              // 首个可见文档行
    pub visible_rows: usize,             // 屏幕容量
    pub total_lines: usize,              // 文档总行数
    pub total_visual_lines: Option<usize>, // 懒计算的总虚拟行数
    pub scroll_y: f64,                   // 视觉行位置（唯一真理源）
}
```

**关键差异：**
- 没有文档锚点 —— `scroll_y` 是一个绝对数值，不关联 buffer 中的任何位置
- `scroll_line` 是 `scroll_y` 的**派生缓存**，每帧由 `update_scroll_line()` 重算
- 当 buffer 内容改变（插入/删除行）时，`scroll_y` 不会自动调整
  - 如果在视口上方插入 10 行，视口不会跟随内容移动
  - 用户看到的是内容上移了 10 行（或者视口跳到了错误位置）

---

## 3. Autoscroll 策略对比

### Zed：显式请求 + 丰富策略

```rust
// crates/editor/src/scroll/autoscroll.rs:15-18
pub enum Autoscroll {
    Next,
    Strategy(AutoscrollStrategy, Option<Anchor>),
}

// crates/editor/src/scroll/autoscroll.rs:99-110
pub enum AutoscrollStrategy {
    Fit,       // 最小滚动量使光标可见
    Newest,    // 只看最新光标
    Center,    // 光标居中
    Focused,   // 光标靠近顶部（留 margin）
    Top,       // 光标在最顶
    Bottom,    // 光标在最底
    TopRelative(ScrollOffset),
    BottomRelative(ScrollOffset),
}
```

**关键设计：**
- Autoscroll 是**显式请求**，不是每帧自动触发
- `request_autoscroll()` 设置标记，`take_autoscroll_request()` 消费标记
- autoscroll 在 layout 阶段**之前**执行（`element.rs:8010-8030`）
- 结果立即反映在当前帧，**不存在延迟**
- 不同操作使用不同策略：
  - 普通输入 → `Autoscroll::fit()`（最小滚动）
  - Go to definition → `Autoscroll::center()` 或 `top_relative()`
  - 搜索结果 → `Autoscroll::newest()`

### Edit+：嵌入渲染循环 + 单一策略

```rust
// crates/app/src/app.rs:1226-1267 (在 shape_visible_lines 末尾)
let cursor_moved = cursor_offset_now != self.last_cursor_offset;
if cursor_moved {
    if let Some(vl_start) = self.visual_line_index.doc_to_visual(cursor_doc_line) {
        let cursor_abs_vl = vl_start + self.cursor_visual_line_in_doc;
        if cursor_abs_vl < first_vl {
            dv.viewport.scroll_to_visual_row(cursor_abs_vl as f64);
        } else if cursor_abs_vl >= last_vl {
            let target = (cursor_abs_vl as f64) - (visible_rows as f64) + 1.0;
            dv.viewport.scroll_to_visual_row(target.max(0.0));
        }
    } else {
        // NOT-IN-VLI: 直接赋值 scroll_line ← BUG
        if cursor_doc_line < range.start {
            dv.viewport.scroll_line = cursor_doc_line;
        } else if cursor_doc_line >= range.end {
            let target = cursor_doc_line.saturating_sub(visible_rows.saturating_sub(1));
            dv.viewport.scroll_line = target;
        }
    }
}
```

**问题：**
1. Autoscroll 逻辑嵌入 `shape_visible_lines` 渲染函数，职责混乱
2. 只有"光标可见"这一种策略，没有 fit/center/top 等区分
3. NOT-IN-VLI 路径直接赋值 `scroll_line`，但 `update_scroll_line()` 会立即覆盖（bug）
4. 每帧都检查 `cursor_moved`，没有显式的 autoscroll 请求机制

---

## 4. 已知 Bug 根因分析

### Bug 1：视图刷新时自动滚动

**现象：** 窗口刷新（resize、重绘等）时视口自动跳动。

**根因链：**

1. `shape_visible_lines()` 末尾的 autoscroll 代码**每帧执行**
2. 判断条件 `cursor_offset_now != self.last_cursor_offset` 在某些情况下误判为 true
   - `last_cursor_offset` 可能未正确更新
   - buffer 编辑后 offset 变化但用户期望视口不动
3. NOT-IN-VLI 路径直接赋值 `scroll_line`（`app.rs:1252,1257`）
4. `update_scroll_line()`（`app.rs:1269`）从 `scroll_y` 重算 `scroll_line`，覆盖了赋值
5. 但 `scroll_y` 未变 → `scroll_line` 回到原值 → 视口不动或跳到错误位置
6. 如果 VLI 本身因帧间重建导致坐标漂移（`vli_relative_coords_cause_drift` 测试证明），`update_scroll_line()` 可能把 `scroll_line` 映射到不同文档行

**对比 Zed：**
- Zed 的 `ScrollAnchor` 使用 buffer 中的稳定锚点，不依赖帧间缓存
- Autoscroll 是显式请求，不会在重绘时意外触发
- 即使 buffer 编辑，锚点自动跟踪，scroll 位置稳定

### Bug 2：方向键按下视口不滚动（长 wrap 行场景）

**现象：** 光标在长 wrap 行内移动，状态栏行号在变，但视口不滚动。

**根因链：**

1. 光标在长 wrap 行（如 30 条虚拟行）的第 11 条虚拟行
2. `cursor_visual_line_in_doc = 11`，`visible_rows = 10`
3. `cursor_abs_vl = vl_start + 11`，`cursor_abs_vl >= last_vl` → 进入 scroll down 分支
4. `scroll_to_visual_row(cursor_abs_vl - visible_rows + 1.0)` 设置 `scroll_y`
5. **下一帧** `update_scroll_line()` 从新 `scroll_y` 推导 `scroll_line`
6. 但 `visible_range()` 按文档行返回，如果 cursor 所在文档行仍在范围内，autoscroll 判定"不需要滚动"
7. **死循环：** 每帧计算相同的 `scroll_y`，每帧被相同的逻辑覆盖

**对比 Zed：**
- Zed 的 `DisplayRow` 已经是 wrap 后的行号，不存在"文档行在范围内但虚拟行在范围外"的情况
- autoscroll 直接操作 `scroll_position.y`（DisplayRow），结果立即生效

### Bug 3：鼠标滚轮不翻页（"闪缩"）

**现象：** 鼠标滚轮滚动时视图区闪烁，但不翻页。

**根因链：**

1. `handle_scroll()` 调用 `viewport.scroll_by(delta)`
2. `scroll_by()` 修改 `scroll_y`，调用 `clamp_scroll_y()`
3. `clamp_scroll_y()` 使用 `total_visual_lines()`，但 `set_total_visual_lines` **从未被调用**
4. `total_visual_lines()` fallback 到 `total_lines`（文档行数），远大于实际虚拟行数
5. `clamp_scroll_y` 的上界过大 → `scroll_y` 可以超过实际内容
6. `update_scroll_line()` 把过大的 `scroll_y` 映射到文档末尾之后
7. `visible_range()` 返回空范围或错误范围 → 渲染为空白

**对比 Zed：**
- Zed 的 `max_scroll_top` 由 `DisplaySnapshot.max_point().row()` 精确计算
- 包含 `ScrollBeyondLastLine` 的三种策略（OnePage / Off / VerticalScrollMargin）
- 不存在"总行数不准"的问题，因为 DisplayMap 统一维护行数

---

## 5. 性能问题分析

### 5.1 Word Wrap 每帧重算

**Edit+：** `shape_visible_lines()` 每帧对所有可见行重新做 word wrap（`app.rs:976-998`）。虽然 shaped text 通过 `shape_cache` 缓存了，但 wrap 结果（`visual_lines: Vec<(start, end, width)>`）不缓存。

**Zed：** `WrapMap` 使用 `SumTree<Transform>` 增量更新。当 buffer 编辑时，只有受影响的区域重新 wrap，其余区域的 wrap 结果通过 `sync()` 增量传播。

```rust
// crates/editor/src/display_map/wrap_map.rs
pub struct WrapMap {
    snapshot: WrapSnapshot,
    pending_edits: VecDeque<(TabSnapshot, Vec<TabEdit>)>,
    interpolated_edits: WrapPatch,
    edits_since_sync: WrapPatch,
    wrap_width: Option<Pixels>,
    background_task: Option<Task<()>>,  // 后台异步 wrap
}
```

**影响：** Edit+ 在大文件 + 窄窗口（大量 wrap）场景下，每帧的 wrap 计算量 = `visible_rows × avg_clusters_per_line`。60fps 下这是主要的 CPU 瓶颈。

### 5.2 VisualLineIndex 每帧重建

**Edit+：** `new_vli` 在 `shape_visible_lines` 中从零构建，每帧只覆盖 `visible_range` 内的文档行。

**Zed：** DisplayMap 的各层（InlayMap、FoldMap、WrapMap 等）都使用 `SumTree` 增量数据结构。编辑操作只修改受影响的节点，其余部分通过树的平衡性保持 O(log n) 更新。

### 5.3 advance_cache 每帧清空重建

**Edit+：** `advance_cache.clear()` 在每帧开始时调用，然后逐行重建。

**影响：** 每帧的 heap 分配（`Vec<(usize, usize, Vec<(usize, f32)>)>` 内部的 `Vec`）导致频繁的 alloc/dealloc。在大 viewport（如 100 行 × 每行 50 clusters）下，每帧 5000+ 个 `(usize, f32)` 的 push。

**优化建议：** 使用 `retain` + 标记过期而非 `clear`，或使用 `SmallVec` 减少 heap 分配。

### 5.4 状态栏字符计数

**Edit+：** `status_bar_text` 每帧 `extract_selected_text` + `from_utf8_lossy` + `chars().count()`（`app.rs:557-576`）。1MB 选区 = 每帧 1MB 拷贝 + UTF-8 扫描。

**Zed：** 选区变化时缓存统计信息，重绘时只读缓存。

---

## 6. `visible_range()` 的设计缺陷

### 问题本质

`visible_range()` 返回 `scroll_line..(scroll_line + visible_rows)`，这是**文档行**范围。但渲染管线的输出是**虚拟行**。

当 `scroll_y = 3.7` 时：
- `scroll_line` = 由 VLI 映射到某个文档行（比如文档行 3）
- `visible_range()` = `3..(3+27)` = 27 条文档行
- 但文档行 3 的前 2 条虚拟行被 skip 了
- 实际渲染的虚拟行数 ≠ `visible_rows`（可能多也可能少）

### Zed 的解决方案

Zed 根本不需要 `visible_range()` 这个概念。渲染管线直接从 `scroll_position.y` 计算 `start_row` 和 `end_row`，两者都是 **DisplayRow**（已经包含 wrap/fold/inlay 的结果）。

```rust
// Zed: start_row 和 end_row 直接从 scroll_position 计算
let start_row = DisplayRow((scroll_position.y + clipped_top_in_lines).floor() as u32);
let end_row = DisplayRow((scroll_position.y + clipped_top_in_lines + visible_height_in_lines).ceil() as u32);
// 两者都是 DisplayRow，直接传给 layout_lines(rows: Range<DisplayRow>)
```

### 修复方向

Edit+ 应该废弃 `visible_range()` 的文档行语义，改为：
1. `shape_visible_lines` 直接从 `scroll_y` 开始，按虚拟行计数
2. 不再遍历"文档行范围"，而是遍历"从 scroll_y 开始的 visible_rows 条虚拟行"
3. 需要一个能从虚拟行号快速定位到 (文档行, 行内偏移) 的索引结构 —— 即 `VisualLineIndex`

---

## 7. 综合对比表

| 维度 | Zed | Edit+ | 评价 |
|------|-----|-------|------|
| **坐标空间** | 单一（DisplayRow） | 双重（doc_line + visual_row） | Zed 更简洁，Edit+ 的双空间是所有 bug 的根源 |
| **滚动锚定** | Anchor + 亚行偏移 | scroll_y 数值 | Zed 的锚点自动跟踪 buffer 编辑；Edit+ 的数值会漂移 |
| **Autoscroll** | 显式请求 + 多策略 | 嵌入渲染循环 + 单策略 | Zed 职责分离清晰；Edit+ 混在一起容易误触发 |
| **Wrap 增量更新** | SumTree + 后台 Task | 每帧全量重算 | Zed 性能远优；Edit+ 大文件下帧率受限 |
| **虚拟行索引** | DisplayMap 各层 SumTree | VisualLineIndex（帧间缓存） | Zed 增量；Edit+ 每帧重建但有 VLI 缓存 |
| **总行数维护** | DisplayMap 统一维护 | total_visual_lines 未设置 | Edit+ 的 clamp_scroll_y 上界不准 |
| **渲染粒度** | DisplayRow 直接渲染 | 文档行 → word wrap → 虚拟行 | Zed 更直接；Edit+ 多了一层转换 |
| **光标→滚动联动** | 独立的 autoscroll pass | shape 末尾的 inline 逻辑 | Zed 可独立测试；Edit+ 与渲染耦合 |
| **Selection 渲染** | 原生支持 | 未实现（stage7 缺口） | Edit+ 需补 |
| **像素级滚动** | scroll_position.y 小数 | scroll_y 小数 + sub_line_pixel_offset | 两者都支持，但 Edit+ 的实现依赖 VLI 坐标准确性 |

---

## 8. 修复方案建议

### 方案 A：渐进修复（推荐）

在现有双坐标空间架构上修 bug，风险最小：

**A1. 修复 NOT-IN-VLI autoscroll（P0）**
- `app.rs:1252,1257` 改为 `scroll_to_visual_row()` 而非直接赋值 `scroll_line`
- 这是"视图刷新自动滚动"的直接修复

**A2. 修复 `set_total_visual_lines` 缺失（P0）**
- 在 `shape_visible_lines` 末尾计算并设置 `total_visual_lines`
- 修复 `clamp_scroll_y` 上界不准导致的过度滚动

**A3. 修复 VLI 坐标漂移（P1）**
- `VisualLineIndex::push()` 的 `vl_start` 必须使用**绝对**视觉行号
- `vli_relative_coords_cause_drift` 测试已证明相对坐标会导致 `scroll_line` 漂移

**A4. Autoscroll 与渲染分离（P1）**
- 将 autoscroll 逻辑从 `shape_visible_lines` 提取为独立函数
- 引入显式 autoscroll 请求机制（类似 Zed 的 `Autoscroll` enum）
- 只在需要时触发，而非每帧检查

### 方案 B：架构重构（长期）

借鉴 Zed 的单一坐标空间设计：

**B1. 统一为虚拟行坐标**
- 废弃 `scroll_line`（文档行），所有滚动/渲染/hit-test 统一使用虚拟行
- `visible_range()` 返回虚拟行范围而非文档行范围
- `shape_visible_lines` 直接从虚拟行号开始渲染

**B2. 引入 DisplayMap 增量更新**
- 将 word wrap 从"每帧重算"改为增量更新
- 使用 `rope` 或 `sum_tree` 维护 wrap 结果
- buffer 编辑时只重算受影响的行

**B3. 稳定锚点**
- 引入类似 Zed 的 `Anchor` 类型，关联 buffer 中的位置
- `scroll_y` 关联到一个锚点而非绝对数值
- buffer 编辑时锚点自动跟踪

### 建议执行顺序

1. A1 + A2 + A3（修复已知 bug，1-2 天）
2. A4（autoscroll 分离，1 天）
3. 测试补强（覆盖所有已知场景）
4. B1（长期重构，需要较大改动，建议在功能稳定后进行）

---

## 9. 边界情况清单

修复后需要测试的边界情况：

| 场景 | 预期行为 | 当前行为 |
|------|---------|---------|
| 长 wrap 行（> visible_rows）内方向键 | 视口跟随光标滚动 | 视口不动 |
| 搜索跳转到屏幕外的长 wrap 行 | 视口跳到目标行 | 可能跳到文档开头 |
| resize 窗口后 wrap 行数变化 | 视口保持当前内容 | 可能跳动 |
| 滚动到文档末尾（最后一行是长 wrap） | 视口停在底部 | 可能过度滚动 |
| 连续滚轮跨文档行边界 | 平滑过渡 | 可能跳动或不翻页 |
| 粘贴大量文本后光标可见 | autoscroll 到光标 | 可能不触发 |
| 光标在 skip 区域（首行前几条虚拟行） | 视口上滚使光标可见 | 光标消失 |
| 文件总行数变化（加载新文件） | 视口重置到开头 | scroll_y 未重置 |

---

## 10. 附录：关键代码路径

### Zed 关键文件
- `crates/editor/src/display_map.rs` — DisplayMap 层次结构定义
- `crates/editor/src/display_map/wrap_map.rs` — WrapMap 增量更新
- `crates/editor/src/scroll.rs` — ScrollAnchor + ScrollManager
- `crates/editor/src/scroll/autoscroll.rs` — Autoscroll 策略
- `crates/editor/src/element.rs:8059-8065` — start_row/end_row 计算

### Edit+ 关键文件
- `crates/app/src/viewport.rs` — Viewport + VisualLineIndex
- `crates/app/src/document_view.rs:878-1270` — shape_visible_lines
- `crates/app/src/app.rs:400-600` — move_cursor_visual
- `crates/app/src/app.rs:1226-1267` — autoscroll（shape 末尾）

### 已有分析文档
- `滚动两类异常根因分析.md` — 两类滚动异常（滚轮闪缩 + 方向键不滚）的详细根因
- `plans_viewport_visual_offset.md` — 虚拟行偏移支持的设计方案
- `plans_viewport_offset_revision.md` — 实现复审与修订方案
