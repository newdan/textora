# Markdown ASCII 框线图右边界吸附实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development`（推荐）或 `executing-plans` 逐任务实施。步骤使用复选框跟踪。

**Goal:** 在不修改 Markdown 源文本的前提下，识别完整轻线矩形，并把同一矩形的右上角、右侧竖线和右下角吸附到同一渲染列。

**Architecture:** `layout::ascii_diagram` 在现有 grapheme 网格上推导 crate-private 的矩形边界吸附元数据，源列继续用于字节范围和高亮。`render` 只对具有吸附元数据的框线单元改用规范列，并为右移角点延长左侧水平连接；普通文字、公开布局 API 和编辑态回退路径保持不变。

**Tech Stack:** Rust、`unicode-segmentation`、现有 `AsciiDiagramRegistry`、`shaping::Shaper`、`ui::core::paint::DrawList`、`textora-markdown`。

## Global Constraints

- 只处理已识别的 Markdown fenced ASCII 框线图；不修改或格式化 Markdown 源文本。
- 同一矩形的右上角、右侧竖线和右下角必须渲染在同一 x 坐标。
- 支持嵌套矩形、上下排列矩形和左右并排矩形；禁止仅凭列距离合并边界。
- 规范右边界只能向右扩展；候选与规范列之间存在非空白字符时不得吸附。
- 普通文字、箭头、非目标框线、源码字节位置、语法高亮和选择映射不得改变。
- 结构不完整、关联歧义、行数不匹配或 shaper 不可用时保持现有安全回退。
- 不改变公开 `LaidOutBlock`、`LaidOutDoc` 或既有 layout/render API 的字段形状和函数签名。
- 光标进入代码块或非空选择与图表相交时，继续使用现有普通文本路径。
- `ui` 不得依赖 Markdown 领域类型；新增数据只存在于 `crates/markdown`。
- 每个任务必须先运行新增测试并确认预期 RED，再写最小实现、确认 GREEN、格式化并提交。
- 保留主工作区中与本任务无关的 `crates/markdown/src/mmf/*` 和其他用户改动，不得暂存、还原或覆盖。

## 文件结构

| 文件 | 职责 |
| --- | --- |
| `crates/markdown/src/layout/ascii_diagram.rs` | 保留源列，推导完整矩形、规范右边界和单元格渲染列；包含结构与降级测试。 |
| `crates/markdown/src/render.rs` | 消费规范渲染列，移动竖向几何并延长右移角点的左侧水平连接；包含真实绘制几何回归。 |

---

### Task 1: 推导完整矩形的规范右边界

**Files:**

- Modify: `crates/markdown/src/layout/ascii_diagram.rs:32-228`
- Test: `crates/markdown/src/layout/ascii_diagram.rs:230-321`

**Interfaces:**

- Produces: `AsciiDiagramCell::render_column(&self) -> usize`，无吸附时返回 `column`。
- Produces: `aligned_column: Option<usize>`，仅为 crate-private 框线几何提供规范列。
- Produces: `align_complete_rectangle_right_edges(rows: &mut [AsciiDiagramRow])`，由 `detect_ascii_diagram` 在检测成功后调用。
- Preserves: `AsciiDiagramCell::column`、`column_width`、`text`、`box_connections` 和所有公共类型/API。

- [ ] **Step 1: 写入代表性、嵌套、并排和安全降级失败测试**

在现有测试模块中加入辅助函数和测试；测试先调用尚不存在的 `render_column()`，因此必须编译失败。

```rust
fn cell_at_source_column(row: &AsciiDiagramRow, column: usize) -> &AsciiDiagramCell {
    row.cells
        .iter()
        .find(|cell| cell.column == column)
        .expect("fixture must contain a cell at the requested source column")
}

fn right_edge_cell(row: &AsciiDiagramRow) -> &AsciiDiagramCell {
    row.cells
        .iter()
        .rev()
        .find(|cell| matches!(cell.text.as_str(), "│" | "┐" | "┘"))
        .expect("fixture row must contain a right edge")
}

#[test]
fn aligns_misaligned_complete_rectangle_right_edges() {
    let diagram = detect_ascii_diagram(&lines(&[
        "┌─ 本地日志（30天滚动）─────────┐",
        "│ · 文件操作（打开/关闭/保存）  │",
        "│ · 模板使用记录               │",
        "└──────────────────────────────┘",
    ]))
    .expect("fixture must be detected as an ASCII diagram");

    let source_columns = diagram
        .rows
        .iter()
        .map(|row| right_edge_cell(row).column)
        .collect::<Vec<_>>();
    let render_columns = diagram
        .rows
        .iter()
        .map(|row| right_edge_cell(row).render_column())
        .collect::<Vec<_>>();

    assert_eq!(source_columns, vec![32, 32, 31, 31]);
    assert_eq!(render_columns, vec![32, 32, 32, 32]);
}

#[test]
fn keeps_nested_rectangle_right_edges_independent() {
    let diagram = detect_ascii_diagram(&lines(&[
        "┌──────────┐",
        "│ ┌──────┐ │",
        "│ │ x  │   │",
        "│ └─────┘  │",
        "└─────────┘",
    ]))
    .expect("fixture must be detected");

    assert_eq!(cell_at_source_column(&diagram.rows[2], 7).render_column(), 9);
    assert_eq!(cell_at_source_column(&diagram.rows[2], 11).render_column(), 11);
    assert_eq!(cell_at_source_column(&diagram.rows[3], 8).render_column(), 9);
    assert_eq!(cell_at_source_column(&diagram.rows[4], 10).render_column(), 11);
}

#[test]
fn keeps_parallel_rectangle_right_edges_independent() {
    let diagram = detect_ascii_diagram(&lines(&[
        "┌────┐  ┌──────┐",
        "│a │    │b   │",
        "└───┘   └─────┘",
    ]))
    .expect("fixture must be detected");

    assert_eq!(cell_at_source_column(&diagram.rows[1], 3).render_column(), 5);
    assert_eq!(cell_at_source_column(&diagram.rows[1], 13).render_column(), 15);
    assert_eq!(cell_at_source_column(&diagram.rows[2], 4).render_column(), 5);
    assert_eq!(cell_at_source_column(&diagram.rows[2], 14).render_column(), 15);
}

#[test]
fn leaves_incomplete_ambiguous_or_nonblank_right_edges_at_source_columns() {
    let incomplete = detect_ascii_diagram(&lines(&["┌────┐", "│x  │"]))
        .expect("thresholds still identify the incomplete fixture as a diagram");
    assert_eq!(right_edge_cell(&incomplete.rows[1]).render_column(), 4);

    let ambiguous =
        detect_ascii_diagram(&lines(&["┌────┐", "│x │", "└──┘", "└───┘"]))
            .expect("thresholds still identify the ambiguous fixture as a diagram");
    assert_eq!(right_edge_cell(&ambiguous.rows[1]).render_column(), 3);

    let blocked = detect_ascii_diagram(&lines(&["┌────┐", "│a │x", "└───┘"]))
        .expect("fixture must be detected");
    assert_eq!(cell_at_source_column(&blocked.rows[1], 3).render_column(), 3);
}
```

- [ ] **Step 2: 运行测试并确认 RED**

Run:

```text
cargo test -p textora-markdown layout::ascii_diagram::tests
```

Expected: FAIL，编译器报告 `AsciiDiagramCell` 不存在 `render_column`。

- [ ] **Step 3: 增加私有渲染列与矩形配对实现**

为 `AsciiDiagramCell` 增加字段和访问器，并在 `grid_row` 中初始化为 `None`：

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AsciiDiagramCell {
    pub(crate) text: String,
    pub(crate) column: usize,
    pub(crate) column_width: usize,
    pub(crate) box_connections: Option<BoxConnections>,
    aligned_column: Option<usize>,
}

impl AsciiDiagramCell {
    pub(crate) fn render_column(&self) -> usize {
        self.aligned_column.unwrap_or(self.column)
    }

    fn align_right_edge_to(&mut self, column: usize) {
        if column < self.column {
            return;
        }
        self.aligned_column = Some(self.aligned_column.map_or(column, |current| current.max(column)));
    }
}
```

`grid_row` 创建单元格时加入：

```rust
aligned_column: None,
```

在 `grid_row` 之后加入以下私有结构和纯函数。角点解析使用栈，保证嵌套框不会被配成内左角到外右角；底边只按相同左列配对，重复候选视为歧义。

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RectangleEdge {
    row_index: usize,
    left_cell_index: usize,
    right_cell_index: usize,
    left_column: usize,
    right_column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CompleteRectangle {
    top: RectangleEdge,
    bottom: RectangleEdge,
}

fn corner_spans(
    row_index: usize,
    row: &AsciiDiagramRow,
    left_corner: &str,
    right_corner: &str,
) -> Vec<RectangleEdge> {
    let mut left_stack = Vec::new();
    let mut spans = Vec::new();

    for (cell_index, cell) in row.cells.iter().enumerate() {
        if cell.text == left_corner {
            left_stack.push((cell_index, cell.column));
            continue;
        }
        if cell.text != right_corner {
            continue;
        }
        let Some((left_cell_index, left_column)) = left_stack.pop() else {
            continue;
        };
        if left_column < cell.column {
            spans.push(RectangleEdge {
                row_index,
                left_cell_index,
                right_cell_index: cell_index,
                left_column,
                right_column: cell.column,
            });
        }
    }

    spans.sort_by_key(|span| span.left_column);
    spans
}

fn matching_bottom_edge(
    rows: &[AsciiDiagramRow],
    top: RectangleEdge,
    used_bottom_edges: &[(usize, usize)],
) -> Option<RectangleEdge> {
    let mut matches = Vec::new();
    for (row_index, row) in rows.iter().enumerate().skip(top.row_index + 1) {
        if corner_spans(row_index, row, "┌", "┐")
            .iter()
            .any(|edge| edge.left_column == top.left_column)
        {
            break;
        }
        matches.extend(
            corner_spans(row_index, row, "└", "┘")
                .into_iter()
                .filter(|edge| edge.left_column == top.left_column)
                .filter(|edge| {
                    !used_bottom_edges.contains(&(edge.row_index, edge.left_cell_index))
                }),
        );
    }
    match matches.as_slice() {
        [edge] => Some(*edge),
        _ => None,
    }
}

fn complete_rectangles(rows: &[AsciiDiagramRow]) -> Vec<CompleteRectangle> {
    let mut rectangles = Vec::new();
    let mut used_bottom_edges = Vec::new();

    for (row_index, row) in rows.iter().enumerate() {
        for top in corner_spans(row_index, row, "┌", "┐") {
            let Some(bottom) = matching_bottom_edge(rows, top, &used_bottom_edges) else {
                continue;
            };
            used_bottom_edges.push((bottom.row_index, bottom.left_cell_index));
            rectangles.push(CompleteRectangle { top, bottom });
        }
    }
    rectangles
}

fn gap_to_column_is_blank(
    row: &AsciiDiagramRow,
    candidate_cell_index: usize,
    target_column: usize,
) -> bool {
    row.cells
        .iter()
        .skip(candidate_cell_index + 1)
        .take_while(|cell| cell.column < target_column)
        .all(|cell| cell.text.chars().all(char::is_whitespace))
}

fn right_edge_candidate(
    row: &AsciiDiagramRow,
    left_column: usize,
    target_column: usize,
) -> Option<usize> {
    row.cells.iter().enumerate().rev().find_map(|(cell_index, cell)| {
        let connections = cell.box_connections?;
        (cell.column > left_column
            && cell.column <= target_column
            && connections.up
            && connections.down
            && gap_to_column_is_blank(row, cell_index, target_column))
        .then_some(cell_index)
    })
}

fn align_complete_rectangle_right_edges(rows: &mut [AsciiDiagramRow]) {
    for rectangle in complete_rectangles(rows) {
        let target_column = rectangle.top.right_column.max(rectangle.bottom.right_column);
        rows[rectangle.top.row_index].cells[rectangle.top.right_cell_index]
            .align_right_edge_to(target_column);
        rows[rectangle.bottom.row_index].cells[rectangle.bottom.right_cell_index]
            .align_right_edge_to(target_column);

        for row in rows
            .iter_mut()
            .take(rectangle.bottom.row_index)
            .skip(rectangle.top.row_index + 1)
        {
            let Some(cell_index) =
                right_edge_candidate(row, rectangle.top.left_column, target_column)
            else {
                continue;
            };
            row.cells[cell_index].align_right_edge_to(target_column);
        }
    }
}
```

在 `detect_ascii_diagram` 通过识别阈值后、计算最终 `column_count` 前调用：

```rust
align_complete_rectangle_right_edges(&mut rows);
```

- [ ] **Step 4: 运行全部新增结构测试并确认 GREEN**

Run:

```text
cargo test -p textora-markdown layout::ascii_diagram::tests
```

Expected: PASS；代表性矩形、嵌套矩形、并排矩形和安全降级测试全部通过，源列保持不变。

- [ ] **Step 5: 运行模块测试、格式检查并提交**

Run:

```text
cargo test -p textora-markdown layout::ascii_diagram::tests
cargo fmt --check
git diff --check
```

Expected: 所有 `layout::ascii_diagram::tests` 通过；格式与空白检查退出码 0。

Commit:

```text
git add crates/markdown/src/layout/ascii_diagram.rs
git commit -m "fix(markdown): infer aligned diagram borders"
```

---

### Task 2: 按规范列绘制竖边与连续角点

**Files:**

- Modify: `crates/markdown/src/render.rs:516-602`
- Test: `crates/markdown/src/render.rs:1504-1574`

**Interfaces:**

- Consumes: `AsciiDiagramCell::render_column(&self) -> usize`。
- Preserves: `render_doc`、`render_layout`、`AsciiDiagramRegistry` 和公开组合入口签名。
- Produces: 对右移且具有 `left` 连接的角点增加 `left_extension_width`，保持顶边/底边连续。

- [ ] **Step 1: 写入真实不等宽矩形的绘制失败测试**

在 render 测试模块中加入：

```rust
#[test]
fn render_snapped_rectangle_uses_one_right_edge_x() {
    let layout = build_laid_out(
        "```\n┌─ 本地日志（30天滚动）─────────┐\n│ · 文件操作（打开/关闭/保存）  │\n│ · 模板使用记录               │\n└──────────────────────────────┘\n```",
    );
    let code_block = layout.doc.blocks.first().expect("fixture has one code block");
    let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
        panic!("fixture must produce a code block");
    };

    let style = default_style();
    let mut draw_list = DrawList::new();
    let mut shaper = shaping::Shaper::new().expect("need shaper for render tests");
    render_doc_with_offset_and_ascii_diagrams(
        &layout.doc,
        &style,
        &mut draw_list,
        0.0,
        600.0,
        0.0,
        0.0,
        Some(&mut shaper),
        &[],
        Some(&layout.ascii_diagrams),
    );

    let right_edge_xs = lines
        .iter()
        .map(|line| {
            vertical_border_xs_for_line(&draw_list, line)
                .last()
                .copied()
                .expect("every fixture row must draw a right edge")
        })
        .collect::<Vec<_>>();
    assert!(
        right_edge_xs.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 0.01),
        "all right-edge segments must share one x coordinate: {right_edge_xs:?}"
    );
}

#[test]
fn render_snapped_rectangle_extends_a_shifted_corner_connection() {
    let layout = build_laid_out("```\n┌────┐\n│x │\n└───┘\n```");
    let code_block = layout.doc.blocks.first().expect("fixture has one code block");
    let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
        panic!("fixture must produce a code block");
    };
    let bottom_line = lines.iter().find(|line| line.text == "└───┘").expect("bottom line");

    let style = default_style();
    let mut draw_list = DrawList::new();
    let mut shaper = shaping::Shaper::new().expect("need shaper for render tests");
    let cell_width =
        code_cell_width(&mut shaper, style.code_font_size, style.code_font_family.as_deref());
    render_doc_with_offset_and_ascii_diagrams(
        &layout.doc,
        &style,
        &mut draw_list,
        0.0,
        600.0,
        0.0,
        0.0,
        Some(&mut shaper),
        &[],
        Some(&layout.ascii_diagrams),
    );

    let has_extended_connection = draw_list.cmds.iter().any(|command| match command {
        DrawCmd::FillRect { rect, .. } => {
            rect.y >= bottom_line.rect.y
                && rect.y + rect.h <= bottom_line.rect.y + bottom_line.rect.h
                && rect.h <= 2.0
                && rect.w > cell_width
        }
        _ => false,
    });
    assert!(has_extended_connection, "a shifted right corner must bridge the added columns");
}
```

- [ ] **Step 2: 运行测试并确认 RED**

Run:

```text
cargo test -p textora-markdown render::tests::render_snapped_rectangle
```

Expected: FAIL；第一项报告多个右边界 x，第二项报告未发现延长的水平连接。

- [ ] **Step 3: 让框线绘制消费规范列并延长左连接**

给 `draw_box_connections` 增加 `left_extension_width: f32` 参数，并替换左连接绘制：

```rust
fn draw_box_connections(
    connections: BoxConnections,
    cell_x: f32,
    left_extension_width: f32,
    line_top: f32,
    cell_width: f32,
    line_height: f32,
    font_size: f32,
    color: [f32; 4],
    dl: &mut DrawList,
) {
    let thickness = (font_size * 0.08).clamp(1.0, 2.0);
    let center_x = cell_x + cell_width * 0.5;
    let center_y = line_top + line_height * 0.5;
    let half = thickness * 0.5;

    if connections.left {
        dl.fill(
            Rect::new(
                cell_x - left_extension_width,
                center_y - half,
                left_extension_width + cell_width * 0.5 + half,
                thickness,
            ),
            color,
        );
    }
    if connections.right {
        dl.fill(
            Rect::new(center_x - half, center_y - half, cell_width * 0.5 + half, thickness),
            color,
        );
    }
    if connections.up {
        dl.fill(Rect::new(center_x - half, line_top, thickness, line_height * 0.5 + half), color);
    }
    if connections.down {
        dl.fill(
            Rect::new(center_x - half, center_y - half, thickness, line_height * 0.5 + half),
            color,
        );
    }
}
```

在 `render_ascii_diagram_row` 中用规范列计算框线 x，并仅对右移单元计算扩展宽度：

```rust
let render_column = cell.render_column();
let cell_x = line.rect.x + ox + render_column as f32 * cell_width;
let allocated_width = cell.column_width as f32 * cell_width;
let left_extension_width = render_column.saturating_sub(cell.column) as f32 * cell_width;
```

调用调整为：

```rust
draw_box_connections(
    connections,
    cell_x,
    left_extension_width,
    line_top,
    allocated_width,
    line.rect.h,
    line.font_size,
    color,
    dl,
);
```

`render_column()` 对普通文字和未吸附框线返回源列，因此其余分支不增加条件或独立路径。

- [ ] **Step 4: 运行新增绘制测试并确认 GREEN**

Run:

```text
cargo test -p textora-markdown render::tests::render_snapped_rectangle
```

Expected: PASS，2 passed；右上角、内容行竖线和右下角共享一个 x，右移角点存在连续水平连接。

- [ ] **Step 5: 运行 Markdown 全量回归和静态检查**

Run:

```text
cargo fmt --check
cargo test -p textora-markdown
cargo check -p textora-markdown
git diff --check
```

Expected: `textora-markdown` unit、integration、doctest 全部通过；check、格式和空白检查退出码 0，无新增 warning。

- [ ] **Step 6: 提交绘制修复**

```text
git add crates/markdown/src/render.rs
git commit -m "fix(markdown): snap diagram right borders"
```

---

## 最终复核

任务级审查全部通过后，由控制代理执行：

```text
./scripts/verify.sh
```

Expected: 退出码 0，末尾输出 `All checks passed! Baseline is trusted.`。

随后对从分支起点到 HEAD 的完整 diff 做最终代码审查，重点确认：

- 源列与规范渲染列没有混用到高亮或 source projection。
- 嵌套/并排矩形没有按距离误合并。
- 右移角点的水平连接连续，且不会覆盖文字。
- 公共 API、普通代码块、活动态和选择态没有回归。
