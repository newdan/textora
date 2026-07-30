# 表格布局修复方案 (Table Layout Bug Fix Plan)

## 问题清单

### Bug 1: 行高不准（即使单行 cell 也有偏差）

Layout 端正确计算了每行高度 `actual_row_h`，但 `LaidOutBlockKind::Table` 不存储行高数据，render 端无法获取，只能硬编码：

| 端 | header 行高 | body 行高 |
|----|------------|----------|
| Layout 实际 | `max_cell_bottom - row_start_y + pad` ≈ 36px | 同 ≈ 36px |
| Render 硬编码 | `line_height + 4.0` = 28px | `line_height + 2.0` = 26px |

每行累积 6~10px 偏差——第 1 行不显，第 3 行已经整体错位。

**影响范围**：任何含表格的文档，无论 cell 内容单行还是多行。

### Bug 2: 单元格文本断行尺子错误（ASCII 树状图等长文本被错误截断/溢出）

`layout_table` 中调用 `ctx.wrap_text(t, font_size)` 来断行，但 `wrap_text` 内部用 `self.available_width()` 作为最大宽度。这个值是 **整张表的可用宽度**（`viewport_w - indent`），不是当前单元格的宽度。

3 列表、600px 视口下：

```
available_width() = 600px   ← wrap_text 用的
cell_inner_w      = 188px   ← 实际单元格可用宽度
```

文本按 600px 断行，却塞进 188px 宽的 cell → 该断的不断，不该断的也可能在奇怪位置被 clip。ASCII 树状图（`├── mod.rs  # comment`）整行长度可能 > 200px，在 188px 的窄 cell 中既填不满也不会正确折行——在 shaper 不可用时还会触发基于字符数的粗略估算，进一步错位。

### Issue 3: 列宽等分，内容密度不均

当前所有列均分宽度。短内容列（数字列）浪费空间，长内容列（文本列、ASCII 树状图）被迫过度折行，拉高总行高。需要改为内容驱动的动态分配。

---

## 修复方案

### Fix 1: 行高数据传递

#### 数据结构变更

**[MODIFY] `crates/markdown/src/layout.rs` — `LaidOutBlockKind::Table`**

```rust
Table {
    columns: usize,
    header: Vec<Vec<LaidOutLine>>,
    rows: Vec<Vec<Vec<LaidOutLine>>>,
    column_widths: Vec<f32>,
    // 新增
    header_height: f32,     // 0.0 = 无 header
    row_heights: Vec<f32>,  // 与 rows 一一对应
}
```

#### Layout 端收集行高

**[MODIFY] `crates/markdown/src/layout.rs` — `layout_table`**

现有代码已经逐行计算 `actual_row_h`，只需在循环中收集：

```rust
// 循环前
let mut body_row_heights: Vec<f32> = Vec::new();

// 每行计算完 actual_row_h 后
if is_header {
    header = row;
    header_actual_h = actual_row_h;
    is_header = false;
} else {
    body_rows.push(row);
    body_rows_h += actual_row_h;
    body_row_heights.push(actual_row_h);  // 新增
}

// 构造时
LaidOutBlockKind::Table {
    // ... 现有字段 ...
    header_height: if header.is_empty() { 0.0 } else { header_actual_h },
    row_heights: body_row_heights,
}
```

#### Render 端使用实际行高

**[MODIFY] `crates/markdown/src/render.rs` — Table 渲染分支**

```rust
LaidOutBlockKind::Table { columns, header, rows, column_widths,
                           header_height, row_heights } => {
    let mut cell_y = y;

    // Header — 用实际高度替代硬编码
    if !header.is_empty() && *header_height > 0.0 {
        dl.fill_rounded(Rect::new(x, cell_y, r.w, *header_height),
                        style.table_header_bg, 0.0);
        for cell_lines in header.iter() {
            for line in cell_lines {
                render_line_with_offset(line, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
            }
        }
        cell_y += *header_height;
        dl.fill(Rect::new(x, cell_y, r.w, 1.0), style.table_border);
    }

    // Body — 遍历 row_heights 替代硬编码
    for (row_idx, (row, &row_h)) in rows.iter().zip(row_heights.iter()).enumerate() {
        cell_y += 2.0;
        if row_idx % 2 == 1 {
            dl.fill_rounded(Rect::new(x, cell_y, r.w, row_h),
                            style.table_stripe_bg, 0.0);
        }
        for cell_lines in row.iter() {
            for line in cell_lines {
                render_line_with_offset(line, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
            }
        }
        cell_y += row_h;
        dl.fill(Rect::new(x, cell_y, r.w, 1.0), style.table_border);
    }

    // Vertical grid lines — 不变
    // ...
}
```

---

### Fix 2: wrap_text 使用正确的单元格宽度

#### 新增显式宽度方法

**[MODIFY] `crates/markdown/src/layout.rs` — `LayoutCtx`**

```rust
/// Word wrap with an explicit maximum width (for use in table cells, etc.).
fn wrap_text_with_width(&mut self, text: &str, font_size: f32, max_w: f32) -> Vec<String> {
    // ... 将现有 wrap_text 逻辑搬过来，但 max_w 从参数获取 ...
}

/// Default: wrap to the full available viewport width.
fn wrap_text(&mut self, text: &str, font_size: f32) -> Vec<String> {
    self.wrap_text_with_width(text, font_size, self.available_width())
}
```

#### layout_table 切换调用

**[MODIFY] `crates/markdown/src/layout.rs` — `layout_table`，cell 文本断行处**

```rust
// 改前
let wrapped = ctx.wrap_text(t, font_size);

// 改后
let wrapped = ctx.wrap_text_with_width(t, font_size, cell_inner_w);
```

其中 `cell_inner_w = cell_w - pad * 2.0`，已经在函数中计算出来。

`wrap_text` 只在 table 路径需要改宽度——段落、标题、列表等场景继续用 `available_width()` 没问题。`wrap_text_with_width` 作为底层，`wrap_text` 保留为便捷包装。

---

### Fix 3: 动态列宽分配

#### 整体流程

```
                  ┌────────────────────┐
                  │  TableWrapper block │
                  └─────────┬──────────┘
                            │
              ┌─────────────▼──────────────┐
              │  Step 1: 测量每列内容需求   │
              │  measure_column_demand()    │
              │  返回 demand[columns]       │
              └─────────────┬──────────────┘
                            │
              ┌─────────────▼──────────────┐
              │  Step 2: 按需求分配列宽     │
              │  allocate_column_widths()   │
              │  返回 column_widths[columns] │
              └─────────────┬──────────────┘
                            │
              ┌─────────────▼──────────────┐
              │  Step 3: 现有 layout_table  │
              │  (用 column_widths 替代等分)│
              └────────────────────────────┘
```

#### 3.1 内容需求测量

**[NEW FN] `crates/markdown/src/layout.rs`**

```rust
/// 测量表格每列的内容宽度需求。
///
/// 对每列取所有 cell 中:
///   - 最长的不可断 token（空格分隔，确保单 token 不被截断）
///   - 与最长整行文本宽度 × 0.6 的较大者
/// 取这两者的 max 作为该列的 demand。
fn measure_column_demand(
    block: &BlockNode,
    columns: usize,
    font_size: f32,
    shaper: Option<&mut Shaper>,
) -> Vec<f32> {
    let mut demand = vec![0.0f32; columns];

    for child in &block.children {
        if !matches!(child.kind, BlockKind::TableRow_) {
            continue;
        }
        for (ci, cell) in child.children.iter().enumerate() {
            if ci >= columns { break; }
            let (texts, _) = collect_text_lines_with_styles(cell);
            for t in &texts {
                if t.is_empty() { continue; }
                // 最长不可断 token
                let max_token_w = shaper.as_mut().map(|s| {
                    s.set_font_size(font_size);
                    t.split(' ')
                        .filter_map(|tok| s.shape(tok).ok().map(|r| r.width))
                        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                        .unwrap_or(0.0)
                }).unwrap_or(0.0);
                // 整行文本宽度
                let full_w = shaper.as_mut().map(|s| {
                    s.set_font_size(font_size);
                    s.shape(t).ok().map(|r| r.width).unwrap_or(0.0)
                }).unwrap_or(t.len() as f32 * font_size * 0.55);
                let d = max_token_w.max(full_w * 0.6);
                if d > demand[ci] {
                    demand[ci] = d;
                }
            }
        }
    }
    demand
}
```

#### 3.2 列宽分配

**[NEW FN] `crates/markdown/src/layout.rs`**

```rust
/// 根据内容需求和可用空间分配列宽。
///
/// 每列保底 `min_col_w`，上限定为可用宽度的 60%。
/// 先分配保底，再按需求比例分配剩余空间。
fn allocate_column_widths(
    demand: &[f32],
    available_w: f32,
    pad: f32,
    min_col_w: f32,
    max_col_w: f32,
) -> Vec<f32> {
    let cols = demand.len();
    if cols == 0 { return vec![]; }

    let total_pad = pad * 2.0 * cols as f32;     // 每列左右 pad
    let net_w = (available_w - total_pad).max(0.0);
    let total_demand: f32 = demand.iter().sum();

    if total_demand <= 0.0 {
        // 全空表——等分
        return vec![net_w / cols as f32; cols];
    }

    let mut widths: Vec<f32> = demand.iter()
        .map(|&d| {
            let w = (net_w * d / total_demand).max(min_col_w).min(max_col_w);
            w + pad * 2.0  // 加回 padding 得到完整列宽（含左右 pad）
        })
        .collect();

    // 约束后的剩余/溢出，按比例再分配
    let allocated: f32 = widths.iter().sum::<f32>() - total_pad;
    let delta = net_w - allocated;

    if delta.abs() > 0.5 {
        let eligible: Vec<usize> = (0..cols)
            .filter(|&i| {
                let w = widths[i] - pad * 2.0;
                if delta > 0.0 { w < max_col_w } else { w > min_col_w }
            })
            .collect();
        let eligible_demand: f32 = eligible.iter().map(|&i| demand[i]).sum();
        if eligible_demand > 0.0 {
            for &i in &eligible {
                let share = delta * demand[i] / eligible_demand;
                widths[i] = (widths[i] + share).max(min_col_w + pad * 2.0).min(max_col_w + pad * 2.0);
            }
        }
    }

    widths
}
```

#### 3.3 layout_table 集成

**[MODIFY] `crates/markdown/src/layout.rs` — `layout_table`**

```rust
fn layout_table(block: &BlockNode, ctx: &mut LayoutCtx, columns: usize) {
    let font_size = ctx.style.body_font_size;
    let line_h = ctx.style.line_height;
    let pad = ctx.style.table_cell_padding;
    let available_w = ctx.available_width().max(20.0);

    // 动态列宽
    let demand = measure_column_demand(block, columns, font_size, ctx.shaper.as_deref_mut());
    let min_col_w = font_size * 3.0;    // 至少放 3 个字符
    let max_col_w = available_w * 0.6;  // 单列不超过 60%
    let column_widths = allocate_column_widths(&demand, available_w, pad, min_col_w, max_col_w);

    // 后续逻辑不变：遍历 rows，用 column_widths[ci] 替代原来的 col_w……
}
```

将原来的：
```rust
let col_w = ctx.available_width() / columns.max(1) as f32;
let column_widths: Vec<f32> = (0..columns).map(|_| col_w).collect();
```
替换为上述动态分配。

---

## 对 ASCII 树状图的影响

修复后的效果：

| 场景 | 修复前 | 修复后 |
|------|--------|--------|
| `├── mod.rs  # Widget trait` 在窄列中 | 文本按 600px 断行（不断），塞进 ~188px → 溢出/裁剪 | 该列获得足够宽度（demand ≈ 350px）→ 完整显示 |
| 列分配 | 数字列 "42" 获得 200px | 数字列 ~90px，文本列 ~380px |
| 行高 | 硬编码 26px/28px → 多行 cell 错位 | 使用实际行高 → 精确对齐 |
| CJK 混合 ASCII 树状图 | 等分列宽 + 错误断行尺子 | 内容驱动列宽 + 正确 cell 宽度断行 |

Box-drawing 字符（U+2500–U+257F，如 `├│└─`）在 `is_cjk_or_fullwidth` 检查中不属于 CJK 区间，与其他 ASCII 一同按 token 处理。Shaper 可用时测量精确宽度，否则退回 `chars * font_size * 0.55` 估算。修复后列宽充足，这些字符不会被意外折断。

---

## 改动总结

| 改动 | 文件 | 类别 | 量 |
|------|------|------|-----|
| `LaidOutBlockKind::Table` 加 `header_height` + `row_heights` | `layout.rs` | 数据结构 | ~3 行 |
| `layout_table` 收集 row_heights | `layout.rs` | 数据流 | ~4 行 |
| Render 端用实际行高 | `render.rs` | Bug Fix | ~15 行 |
| 新增 `wrap_text_with_width` | `layout.rs` | 新增函数 | ~5 行包装 |
| `layout_table` 传 `cell_inner_w` | `layout.rs` | Bug Fix | 改 1 行 |
| 新增 `measure_column_demand` | `layout.rs` | 新功能 | ~35 行 |
| 新增 `allocate_column_widths` | `layout.rs` | 新功能 | ~35 行 |
| `layout_table` 集成动态列宽 | `layout.rs` | 集成 | 删 2 行 + 加 4 行 |

所有改动集中在 `layout.rs` 和 `render.rs`，不影响其他 crate。数据流方向：`measure → allocate → layout → render`，每步职责单一。
