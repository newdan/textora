# DisplayRow 统一坐标空间 — 详细实施方案

> 目标：借鉴 Zed 的 DisplayRow 设计，消除 Edit+ 的双坐标空间问题，
> 从根本上修复"视图刷新自动滚动"、"长行方向键不滚动"等已知 bug。

---

## 1. 设计目标

### 1.1 核心问题

当前 Edit+ 同时使用两个坐标空间：

| 坐标 | 用途 | 来源 |
|------|------|------|
| `scroll_y: f64` | 滚动位置（虚拟行） | 唯一真理源 |
| `scroll_line: usize` | 首个可见文档行 | 每帧从 `scroll_y` 推导 |

渲染管线在两个空间之间反复切换，导致：
- `visible_range()` 返回文档行范围，但渲染输出是虚拟行
- autoscroll 在虚拟行空间判断"光标是否可见"，但在文档行空间执行滚动
- NOT-IN-VLI 路径直接赋值 `scroll_line`，被 `update_scroll_line()` 立即覆盖

### 1.2 目标状态

**单一坐标空间：`DisplayRow`（虚拟行）**

- 滚动位置、可见范围、autoscroll、hit-test 全部使用 `DisplayRow`
- `visible_range()` 返回 `Range<DisplayRow>` 而非 `Range<usize>`（文档行）
- 文档行仅在"需要读取 buffer 内容"时使用（从 DisplayRow 反查）
- autoscroll 判断与执行统一在 DisplayRow 空间

### 1.3 与 Zed 的差异

Zed 有完整的 DisplayMap 层次（InlayMap → FoldMap → TabMap → WrapMap → BlockMap → DisplayMap），
每层用 SumTree 增量更新。Edit+ 目前只有 word-wrap，不需要这么复杂的架构。

**本次迁移只做坐标空间统一，不引入 SumTree / DisplayMap 层次结构。**
Word wrap 仍按现有方式（每帧对可见行重算），后续再考虑增量优化。

---

## 2. 类型设计

### 2.1 `DisplayRow` 新类型

```rust
// crates/app/src/viewport.rs

/// 表示 word-wrap 之后的虚拟行号。
/// 0 = 屏幕最顶部的虚拟行，每条文档行可能对应 1~N 条 DisplayRow。
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialOrd, PartialEq, Hash)]
pub struct DisplayRow(pub u32);

impl DisplayRow {
    pub fn as_f64(self) -> f64 { self.0 as f64 }
    pub fn as_usize(self) -> usize { self.0 as usize }
    pub fn saturating_sub(self, rhs: u32) -> Self {
        DisplayRow(self.0.saturating_sub(rhs))
    }
    pub fn next(self) -> Self {
        DisplayRow(self.0 + 1)
    }
}

impl std::ops::Add<u32> for DisplayRow {
    type Output = Self;
    fn add(self, rhs: u32) -> Self { DisplayRow(self.0 + rhs) }
}

impl std::ops::Sub<u32> for DisplayRow {
    type Output = Self;
    fn sub(self, rhs: u32) -> Self { DisplayRow(self.0 - rhs) }
}

impl std::ops::AddAssign<u32> for DisplayRow {
    fn add_assign(&mut self, rhs: u32) { self.0 += rhs; }
}

impl std::ops::SubAssign<u32> for DisplayRow {
    fn sub_assign(&mut self, rhs: u32) { self.0 -= rhs; }
}

impl From<u32> for DisplayRow {
    fn from(v: u32) -> Self { DisplayRow(v) }
}
```

### 2.2 `DocLine` 新类型（可选，增强类型安全）

```rust
/// 文档行号（wrap 前）。仅在需要读取 buffer 内容时使用。
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialOrd, PartialEq, Hash)]
pub struct DocLine(pub u32);
```

**决策：** 暂不引入 `DocLine`，因为 `DocumentView` 的大部分 API 已经用 `usize` 表示文档行号，
引入新类型会导致大量适配改动。`DisplayRow` 是本次的核心改动，`DocLine` 可以后续再加。

---

## 3. Viewport 改造

### 3.1 字段变更

```rust
// 改造前
pub struct Viewport {
    pub scroll_line: usize,              // 文档行
    pub visible_rows: usize,
    pub total_lines: usize,
    pub total_visual_lines: Option<usize>,
    pub scroll_y: f64,
}

// 改造后
pub struct Viewport {
    /// 滚动位置：首个可见虚拟行（DisplayRow 单位，支持小数实现亚行像素滚动）。
    /// 这是唯一的滚动真理源。
    pub scroll_top: f64,                 // renamed from scroll_y
    /// 屏幕能容纳的虚拟行数。
    pub visible_rows: usize,
    /// 文档总行数（用于 doc_line 级别的 clamp，如 set_total_lines）。
    pub total_lines: usize,
    /// 总虚拟行数（lazy，用于 clamp_scroll_top）。
    pub total_visual_lines: Option<usize>,
    /// 缓存：scroll_top 对应的首个可见文档行。
    /// 由 update_first_visible_doc_line() 从 scroll_top + VLI 推导。
    /// 不参与滚动逻辑，仅供渲染循环使用。
    pub(crate) first_visible_doc_line: usize,
}
```

**移除的字段：**
- `scroll_line` → 改名为 `first_visible_doc_line`，降级为缓存
- `scroll_visual_offset` → 已在之前移除（由 `scroll_y` 的小数部分替代）
- `scroll_line_visual_count` → 已在之前移除

**新增的方法：**

```rust
impl Viewport {
    /// 首个可见虚拟行（整数部分）。
    pub fn first_visible_row(&self) -> DisplayRow {
        DisplayRow(self.scroll_top.floor() as u32)
    }

    /// 亚行像素偏移（负值，表示首行向上偏移的量）。
    pub fn sub_line_pixel_offset(&self, line_height: f32) -> f32 {
        -(self.scroll_top.fract() as f32 * line_height)
    }

    /// 可见范围：DisplayRow 空间。
    /// 返回 [first_visible_row, first_visible_row + visible_rows)。
    pub fn visible_display_range(&self) -> std::ops::Range<DisplayRow> {
        let start = self.first_visible_row();
        let end = DisplayRow((self.scroll_top + self.visible_rows as f64).ceil() as u32);
        start..end
    }

    /// 保留旧接口用于渲染循环的文档行迭代。
    /// 返回的是文档行范围（近似值，可能偏大），渲染循环靠 B2 break guard 截断。
    pub fn visible_doc_line_range(&self) -> std::ops::Range<usize> {
        self.first_visible_doc_line
            ..(self.first_visible_doc_line + self.visible_rows).min(self.total_lines)
    }

    /// 从 scroll_top + VLI 更新 first_visible_doc_line 缓存。
    pub fn update_first_visible_doc_line(&mut self, index: &VisualLineIndex) {
        if index.is_empty() { return; }
        let row = self.first_visible_row().as_usize();
        let (doc_line, _offset) = index.visual_to_doc(row);
        self.first_visible_doc_line = doc_line;
    }
}
```

### 3.2 滚动 API 变更

```rust
impl Viewport {
    /// 按虚拟行滚动（正=下，负=上）。鼠标滚轮 / 触控板使用。
    pub fn scroll_by(&mut self, delta: f64) {
        self.scroll_top = (self.scroll_top + delta).max(0.0);
        self.clamp_scroll_top();
    }

    /// 滚动到指定虚拟行位置。autoscroll / goto 使用。
    pub fn scroll_to_row(&mut self, row: f64) {
        self.scroll_top = row.max(0.0);
        self.clamp_scroll_top();
    }

    /// clamp scroll_top 使不超过内容底部。
    fn clamp_scroll_top(&mut self) {
        let max = self.total_visual_lines()
            .saturating_sub(self.visible_rows) as f64;
        if self.scroll_top > max {
            self.scroll_top = max.max(0.0);
        }
    }
}
```

**移除的 API：**
- `scroll_down(delta)` / `scroll_up(delta)` / `scroll_to(line)` — 文档行级 API，不再需要
- `update_scroll_line(index)` → 改名为 `update_first_visible_doc_line(index)`
- `visible_range()` → 拆为 `visible_display_range()` + `visible_doc_line_range()`

### 3.3 VisualLineIndex 变更

`VisualLineIndex` 保持不变。它的输入/输出已经是 (doc_line, visual_line_start) 对，
与 DisplayRow 兼容。只需确保 `push()` 使用**绝对**视觉行号（修复已知的坐标漂移 bug）。

---

## 4. 渲染管线改造（shape_visible_lines）

### 4.1 当前流程

```
1. update_scroll_line(scroll_y → scroll_line)        // 坐标转换
2. range = visible_range()                            // 文档行范围
3. for doc_line in range:                             // 遍历文档行
     shape → word_wrap → visual_lines
     skip_visual = (i==0) ? compute_skip() : 0       // 首行跳过
     for vl in visual_lines[skip_visual..]:
       advance_cache.push(...)
       render_glyphs(...)
     if visual_line_counter >= visible_rows: break    // B2 截断
4. autoscroll (在 DisplayRow 空间判断，在 doc_line 空间执行)  ← bug
5. update_scroll_line(scroll_y → scroll_line)         // 再次同步
```

### 4.2 改造后流程

```
1. update_first_visible_doc_line(scroll_top → first_visible_doc_line)
2. range = visible_doc_line_range()                   // 文档行范围（近似）
3. skip_visual = compute_skip_from_scroll_top()       // 从 scroll_top 计算首行跳过数
4. for doc_line in range:                             // 遍历文档行
     shape → word_wrap → visual_lines
     skip = (i==0) ? skip_visual : 0
     for vl in visual_lines[skip..]:
       advance_cache.push(...)
       render_glyphs(...)
     if visual_line_counter >= visible_rows: break    // B2 截断
5. autoscroll (统一在 DisplayRow 空间)                 ← 修复
6. update_first_visible_doc_line(scroll_top → first_visible_doc_line)
```

**关键变化：**
- 步骤 1 和 6：方法名变更，逻辑不变
- 步骤 3：`skip_visual` 的计算保持不变（已经是基于 scroll_top 的）
- 步骤 5：autoscroll 重写（见第 5 节）
- `visible_range()` 改为 `visible_doc_line_range()`，语义更清晰

### 4.3 advance_cache 变更

```rust
// 改造前
advance_cache: Vec<(usize, usize, Vec<(usize, f32)>)>,
//                doc_line, vl_byte_start, [(cluster_end_byte, pixel_x)]

// 改造后：增加 DisplayRow 信息
advance_cache: Vec<AdvanceCacheEntry>,

struct AdvanceCacheEntry {
    display_row: DisplayRow,        // 该虚拟行的绝对 DisplayRow
    doc_line: usize,                // 所属文档行
    vl_byte_start: usize,           // 虚拟行在文档行内的字节起始
    clusters: Vec<(usize, f32)>,    // [(cluster_end_byte, pixel_x)]
}
```

**好处：**
- hit_test 直接返回 `DisplayRow`，不需要再通过 `advance_cache` 索引反推
- move_cursor_visual 的 4a 分支直接用 `DisplayRow` 索引
- selection_vertices 直接用 `DisplayRow` 计算 y 坐标

---

## 5. Autoscroll 重写

### 5.1 当前问题

```rust
// app.rs:1226-1267 — 当前代码（简化）
if cursor_moved {
    if cursor_doc_line IN vli {
        // DisplayRow 空间判断 ✓
        cursor_abs_vl = vl_start + cursor_visual_line_in_doc;
        if cursor_abs_vl < first_vl { scroll_to_visual_row(...) }
        else if cursor_abs_vl >= last_vl { scroll_to_visual_row(...) }
    } else {
        // 文档行空间执行 ✗ — scroll_line 被 update_scroll_line 覆盖
        scroll_line = cursor_doc_line;  // NO-OP!
    }
}
```

### 5.2 改造后

```rust
// 统一在 DisplayRow 空间
fn autoscroll_cursor(&mut self) {
    let cursor_offset_now = self.doc_view().cursor_offset;
    let cursor_moved = cursor_offset_now != self.last_cursor_offset;
    if !cursor_moved { return; }

    let cursor_doc_line = self.doc_view().cursor_line();
    let cursor_vl_in_doc = self.cursor_visual_line_in_doc;

    // 尝试从 VLI 获取绝对 DisplayRow
    let cursor_abs_row: DisplayRow = if let Some(vl_start) =
        self.visual_line_index.doc_to_visual(cursor_doc_line)
    {
        DisplayRow((vl_start + cursor_vl_in_doc) as u32)
    } else {
        // cursor 不在当前 VLI 范围内 — 用文档行近似
        // （文档行号 ≈ 虚拟行号的下界，因为每行至少 1 条虚拟行）
        DisplayRow(cursor_doc_line as u32)
    };

    let first_visible = self.viewport().first_visible_row();
    let last_visible = first_visible + self.viewport().visible_rows as u32;

    if cursor_abs_row < first_visible {
        // 光标在视口上方 → 向上滚动
        self.doc_view_mut().viewport.scroll_to_row(cursor_abs_row.as_f64());
    } else if cursor_abs_row >= last_visible {
        // 光标在视口下方 → 向下滚动
        let target = cursor_abs_row.as_f64() - self.viewport().visible_rows as f64 + 1.0;
        self.doc_view_mut().viewport.scroll_to_row(target.max(0.0));
    }
    // else: 光标在视口内，不需要滚动
}
```

**关键改进：**
1. **统一坐标空间** — 判断和执行都在 DisplayRow 空间
2. **NOT-IN-VLI 不再是特殊分支** — 用文档行号近似（每行至少 1 条虚拟行），
   虽然不精确但方向正确，不会死循环
3. **直接调用 `scroll_to_row()`** — 修改 `scroll_top`（唯一真理源），
   不再有 `scroll_line` 被覆盖的问题
4. **不再嵌入 `shape_visible_lines`** — 提取为独立函数，职责分离

### 5.3 与 ensure_cursor_visible 的关系

`DocumentView::ensure_cursor_visible()` 是另一个 autoscroll 入口（在 `execute_edit_command` 中调用）。
它当前使用文档行空间：

```rust
pub fn ensure_cursor_visible(&mut self) {
    let line = self.cursor_line();
    let range = self.viewport.visible_range();
    if line < range.start {
        self.viewport.scroll_to(line);      // 文档行级
    } else if line >= range.end {
        self.viewport.scroll_to(line - visible_rows + 1);
    }
}
```

**改造方案：** 移除 `ensure_cursor_visible()`，统一由 `autoscroll_cursor()` 处理。
`execute_edit_command` 中不再调用 `ensure_cursor_visible()`，改为设置 `cursor_moved` 标记，
让下一帧的 `autoscroll_cursor()` 处理。

---

## 6. move_cursor_visual 改造

### 6.1 当前问题

`move_cursor_visual`（`app.rs:400-600`）处理 word-wrap 下的上下箭头。
三个分支都使用 `advance_cache` 索引（隐式 DisplayRow），但 4b/4c 在超出 advance_cache 范围时
退回文档行空间操作。

### 6.2 改造方案

**4a 分支（target 在 advance_cache 内）：** 改动最小。
`advance_cache` 索引本身就是 DisplayRow（屏幕内虚拟行），只需用新结构的 `display_row` 字段。

**4b 分支（target 在视口上方）：**
```rust
// 改造前：直接修改 scroll_visual_offset（已移除）
// 改造后：修改 scroll_top
if target_vis < 0 {
    let abs_target_row = self.viewport().first_visible_row()
        .saturating_sub((-target_vis) as u32);
    // 用 sticky_x 在目标虚拟行上定位列
    // ...
    // 滚动视口
    self.doc_view_mut().viewport.scroll_to_row(abs_target_row.as_f64());
}
```

**4c 分支（target 在视口下方）：**
```rust
// 改造后
if target_vis >= advance_cache.len() {
    let abs_target_row = self.viewport().first_visible_row()
        + target_vis as u32;
    // 用 sticky_x 在目标虚拟行上定位列
    // ...
    // 滚动视口
    let target_scroll = abs_target_row.as_f64()
        - self.viewport().visible_rows as f64 + 1.0;
    self.doc_view_mut().viewport.scroll_to_row(target_scroll.max(0.0));
}
```

---

## 7. hit_test 改造

### 7.1 当前实现

```rust
fn hit_test(&self, px: f32, py: f32) -> (usize, usize) {
    let vis_line = ((py - sub_line_offset) / line_height) as usize;
    let (doc_line, vl_byte_start, clusters) = &self.advance_cache[vis_line];
    // ... binary search in clusters ...
    (doc_line, byte_offset)
}
```

### 7.2 改造后

```rust
fn hit_test(&self, px: f32, py: f32) -> HitResult {
    let vis_line = ((py - sub_line_offset) / line_height) as usize;
    let entry = &self.advance_cache[vis_line];
    // ... binary search in clusters ...
    HitResult {
        display_row: entry.display_row,
        doc_line: entry.doc_line,
        byte_offset,
    }
}
```

返回 `DisplayRow` 使得后续的光标移动可以直接在 DisplayRow 空间操作。

---

## 8. 分阶段实施计划

### 阶段 1：引入 DisplayRow 类型 + Viewport 字段改造

**文件：** `crates/app/src/viewport.rs`

**改动：**
1. 新增 `DisplayRow` 类型定义（§2.1）
2. `Viewport` 字段重命名：`scroll_y` → `scroll_top`
3. 新增方法：`first_visible_row()`, `sub_line_pixel_offset()`, `visible_display_range()`
4. 新增方法：`scroll_to_row()`, `update_first_visible_doc_line()`
5. 保留旧方法的兼容别名（deprecated），标记 `scroll_line` 为 `pub(crate)`
6. 保留 `visible_doc_line_range()` 作为渲染循环的临时接口
7. 修复 `clamp_scroll_top()` 使用正确的 `total_visual_lines`
8. 更新所有 viewport.rs 内的测试

**测试：**
- `DisplayRow` 算术运算
- `first_visible_row()` 小数截断
- `visible_display_range()` 正确性
- `scroll_to_row()` + `clamp_scroll_top()` 边界
- `update_first_visible_doc_line()` 绝对坐标不漂移

**编译验证：** `cargo check -p edit-plus-app`

---

### 阶段 2：渲染管线适配

**文件：** `crates/app/src/app.rs`（shape_visible_lines）

**改动：**
1. `scroll_y` → `scroll_top` 全局替换
2. `update_scroll_line()` → `update_first_visible_doc_line()`
3. `visible_range()` → `visible_doc_line_range()`
4. advance_cache 结构改为 `AdvanceCacheEntry`（§4.3）
5. hit_test 返回 `HitResult`（含 `DisplayRow`）
6. 更新 `cursor_vertices` / `selection_vertices` 使用新 advance_cache 结构

**测试：**
- 现有渲染相关测试全部通过
- advance_cache 长度 = visible_rows（截断正确）
- hit_test 返回的 DisplayRow 与 advance_cache 一致

**编译验证：** `cargo check -p edit-plus-app`

---

### 阶段 3：Autoscroll 重写

**文件：** `crates/app/src/app.rs`

**改动：**
1. 从 `shape_visible_lines` 提取 autoscroll 逻辑为独立函数 `autoscroll_cursor()`
2. 实现统一 DisplayRow 空间的 autoscroll（§5.2）
3. 移除 NOT-IN-VLI 分支的 `scroll_line = ...` 赋值
4. 移除 `DocumentView::ensure_cursor_visible()` 和 `ensure_cursor_visible_sync()`
5. `execute_edit_command` 中移除所有 `ensure_cursor_visible()` 调用
6. 设置 `cursor_moved` 标记，由 `autoscroll_cursor()` 在下一帧处理

**测试：**
- `cursor_jumps_outside_vli_scrolls_correctly` — 搜索跳转
- `long_wrap_line_cursor_scroll_follows` — 长行方向键
- `mouse_wheel_does_not_trigger_autoscroll` — 滚轮不触发 autoscroll
- `resize_does_not_auto_scroll` — resize 不触发自动滚动

**编译验证：** `cargo check -p edit-plus-app && cargo test -p edit-plus-app --lib`

---

### 阶段 4：move_cursor_visual 适配

**文件：** `crates/app/src/app.rs`

**改动：**
1. 4a 分支：使用新 advance_cache 的 `display_row` 字段
2. 4b 分支：用 `scroll_to_row()` 替代 `scroll_visual_offset -= 1`
3. 4c 分支：用 `scroll_to_row()` 替代 `scroll_visual_offset += 1`
4. 确保 sticky_x 在 DisplayRow 空间正确传递

**测试：**
- `up_arrow_in_long_wrap_line_scrolls_up` — 长行上箭头
- `down_arrow_in_long_wrap_line_scrolls_down` — 长行下箭头
- `sticky_x_preserved_across_wrap_lines` — 列保持

**编译验证：** `cargo test -p edit-plus-app --lib`

---

### 阶段 5：清理旧接口

**文件：** `crates/app/src/viewport.rs`, `crates/app/src/document_view.rs`, `crates/app/src/app.rs`

**改动：**
1. 移除 `Viewport::scroll_line` 字段（改为 `first_visible_doc_line: usize`）
2. 移除 `Viewport::scroll_y` 别名
3. 移除 `Viewport::visible_range()` → 只保留 `visible_doc_line_range()` + `visible_display_range()`
4. 移除 `DocumentView::ensure_cursor_visible()` / `ensure_cursor_visible_sync()`
5. 清理 `app.rs` 中所有 `scroll_line` 直接访问
6. 更新 `document_view.rs` 中的 `visible_line()` / `visible_lines()` / `visible_line_count()` 使用 `visible_doc_line_range()`
7. 更新所有测试

**测试：** 全量测试通过 `cargo test -p edit-plus-app --lib`

---

### 阶段 6：total_visual_lines 修复

**文件：** `crates/app/src/app.rs`, `crates/app/src/viewport.rs`

**改动：**
1. 在 `shape_visible_lines` 末尾，用 `new_vli.total_visual_lines()` 更新 viewport 的 `total_visual_lines`
2. 确保 `clamp_scroll_top()` 使用正确的上界
3. 修复过度滚动（鼠标滚轮滚过文档末尾）

**测试：**
- `scroll_past_document_end_clamps_correctly`
- `total_visual_lines_updated_after_shape`

---

### 阶段 7：回归测试 + 手动验证

**自动化测试：**

| 测试名 | 覆盖场景 |
|--------|---------|
| `display_row_arithmetic` | DisplayRow 加减法 |
| `viewport_scroll_to_row_basic` | 基本滚动 |
| `viewport_scroll_to_row_clamp` | 边界 clamp |
| `visible_display_range_matches_visible_rows` | 可见范围大小 |
| `autoscroll_cursor_below_viewport` | 光标在视口下方 |
| `autoscroll_cursor_above_viewport` | 光标在视口上方 |
| `autoscroll_cursor_in_viewport_noop` | 光标在视口内不滚动 |
| `autoscroll_not_in_vli_scrolls` | 光标不在 VLI 时也能滚动 |
| `mouse_wheel_no_autoscroll` | 滚轮不触发 autoscroll |
| `resize_no_auto_scroll` | resize 不自动滚动 |
| `long_wrap_line_down_arrow_scrolls` | 长行下箭头滚动 |
| `long_wrap_line_up_arrow_scrolls` | 长行上箭头滚动 |
| `hit_test_returns_display_row` | hit_test 返回 DisplayRow |
| `advance_cache_entry_has_display_row` | advance_cache 含 DisplayRow |
| `total_visual_lines_clamps_scroll` | 总虚拟行数正确 clamp |

**手动验证：**
1. 打开有长行的文件，word-wrap 开启
2. 用下箭头在长行内移动，确认视口跟随滚动
3. 用鼠标滚轮滚动，确认不闪烁、不跳动
4. resize 窗口，确认视口不自动跳动
5. 搜索跳转到屏幕外的长行，确认视口正确跳转
6. 滚动到文档末尾，确认不过度滚动

---

## 9. 风险点与缓解

### 9.1 文档行近似的精度

**风险：** NOT-IN-VLI 路径用 `DisplayRow(cursor_doc_line)` 近似光标位置。
当 cursor_doc_line 之前有 wrap 行时，这个近似偏小（因为 wrap 行产生多条 DisplayRow）。

**缓解：**
- 近似值偏小 → autoscroll 会多滚一点（光标可能不在屏幕正中），但不会死循环
- 下一帧 shape 后，cursor_abs_row 会从 VLI 获取精确值，autoscroll 会微调
- 两帧内收敛，用户不可感知

### 9.2 旧测试兼容

**风险：** 移除 `scroll_line` / `visible_range()` 后，现有测试大量依赖这些接口。

**缓解：**
- 阶段 1 保留旧接口的 deprecated 别名
- 阶段 2-4 逐步迁移测试
- 阶段 5 最终清理

### 9.3 ensure_cursor_visible 移除的副作用

**风险：** `execute_edit_command` 中每个命令都调用 `ensure_cursor_visible()`。
移除后如果 `autoscroll_cursor()` 没有正确触发，光标可能离开屏幕。

**缓解：**
- `execute_edit_command` 返回 `true` 时，`needs_redraw = true`，下一帧会调用 `render()`
- `render()` 内调用 `shape_visible_lines()`，末尾调用 `autoscroll_cursor()`
- 延迟一帧，但用户不可感知（60fps = 16.7ms/帧）

### 9.4 VisualLineIndex 坐标漂移

**风险：** `push()` 使用相对视觉行号时，`visual_to_doc()` 返回错误结果。

**缓解：**
- `vli_absolute_coords_wrapped_no_drift` 测试已验证绝对坐标正确
- `vli_relative_coords_cause_drift` 测试验证相对坐标的 bug
- `shape_visible_lines` 中 `push()` 使用 `new_vli.total_visual_lines()` 作为 `vl_start`
  — 这是绝对坐标，因为 `new_vli` 从头构建

---

## 10. 文件改动汇总

| 文件 | 阶段 | 改动量 | 说明 |
|------|------|--------|------|
| `crates/app/src/viewport.rs` | 1, 5, 6 | **大** | DisplayRow 类型 + Viewport 重构 |
| `crates/app/src/app.rs` | 2, 3, 4 | **大** | shape_visible_lines + autoscroll + move_cursor_visual |
| `crates/app/src/document_view.rs` | 5 | **中** | 移除 ensure_cursor_visible，更新 visible_* API |
| `crates/app/src/input.rs` | 3 | **小** | 移除 ensure_cursor_visible 调用（如果有的话） |

**总改动量：** ~500-800 行改动，主要是重命名 + 接口调整 + autoscroll 重写。

---

## 11. 与已有文档的关系

| 已有文档 | 本方案的关系 |
|---------|------------|
| `plans_viewport_visual_offset.md` | 本方案替代其阶段 1-4（scroll_visual_offset 已被 scroll_top 替代） |
| `plans_viewport_offset_revision.md` | 本方案修复其 A1/B1/B2（通过统一坐标空间从根本上消除） |
| `滚动两类异常根因分析.md` | 本方案修复其两个症状的共同症结（visible_range 不补偿 offset） |
| `stage7_review.md` | 无直接关系，但 autoscroll 分离后更容易实现选区渲染 |

