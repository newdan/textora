# ASCII 图水平箭头对齐 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 ASCII 图预览中的 `────────→` 与 `←────────` 使用和框线相同的水平中心线绘制。

**Architecture:** 保留现有网格布局和源码映射，只在 `render_ascii_diagram_row` 中根据相邻单元格识别连续水平箭头。箭身使用 `FillRect`，箭头头部使用 `FillTriangle`，两者和 `draw_box_connections` 共用行框垂直中心；独立文本箭头仍走字体渲染。

**Tech Stack:** Rust、`ui::core::paint::DrawList`、`textora-markdown` 单元测试。

## Global Constraints

- 只处理 ASCII 图预览；不得修改源 Markdown、布局列数、复制内容或编辑态。
- `→` 仅在左邻格为 `─` 时几何化；`←` 仅在右邻格为 `─` 时几何化。
- 竖向箭头及独立文本箭头不在本次范围内。
- 不新增依赖，不跨越 `markdown -> ui -> app` 的既有依赖方向。

---

### Task 1: 连续水平箭头几何渲染

**Files:**
- Modify: `crates/markdown/src/render.rs`
- Test: `crates/markdown/src/render.rs`

**Interfaces:**
- Consumes: `AsciiDiagramRow::cells`、`AsciiDiagramCell::{text, render_column}`、`DrawList::{fill, fill_triangle}`。
- Produces: 私有 `HorizontalArrowDirection`、连续箭头识别函数和几何绘制函数；无公共 API 变化。

- [ ] **Step 1: 写右向、左向与独立文本箭头的失败回归测试**

在 `render.rs` 的测试模块加入一个包含三种情况的 ASCII 图 fixture，并加入按行提取三角形的 helper：

```rust
const HORIZONTAL_ARROW_DIAGRAM: &str = r#"```
┌────────────┐
│ ────────→  │
│ ←────────  │
│ DSL → add  │
└────────────┘
```"#;

fn fill_triangles_for_line(draw_list: &DrawList, line: &LaidOutLine) -> Vec<[[f32; 2]; 3]> {
    draw_list
        .cmds
        .iter()
        .filter_map(|command| match command {
            DrawCmd::FillTriangle { p0, p1, p2, .. }
                if [p0, p1, p2]
                    .into_iter()
                    .all(|point| line.rect.y <= point[1] && point[1] <= line.rect.y + line.rect.h) =>
            {
                Some([*p0, *p1, *p2])
            }
            _ => None,
        })
        .collect()
}

#[test]
fn render_horizontal_connector_arrows_share_box_line_center() {
    const GEOMETRY_EPSILON: f32 = 0.01;

    let layout = build_laid_out(HORIZONTAL_ARROW_DIAGRAM);
    let code_block = layout.doc.blocks.first().expect("fixture has one code block");
    let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
        panic!("fixture must produce a code block");
    };
    let draw_list = render_laid_out(&layout, 600.0);

    let right_arrow = fill_triangles_for_line(&draw_list, &lines[1]);
    let left_arrow = fill_triangles_for_line(&draw_list, &lines[2]);
    assert_eq!(right_arrow.len(), 1, "right connector must draw one arrowhead");
    assert_eq!(left_arrow.len(), 1, "left connector must draw one arrowhead");

    for (line, triangle) in [(&lines[1], right_arrow[0]), (&lines[2], left_arrow[0])] {
        let minimum_y = triangle.iter().map(|point| point[1]).fold(f32::INFINITY, f32::min);
        let maximum_y = triangle.iter().map(|point| point[1]).fold(f32::NEG_INFINITY, f32::max);
        let arrow_center_y = (minimum_y + maximum_y) * 0.5;
        let box_line_center_y = line.rect.y + line.rect.h * 0.5;
        assert!((arrow_center_y - box_line_center_y).abs() < GEOMETRY_EPSILON);
    }

    assert!(right_arrow[0][0][0] > right_arrow[0][1][0]);
    assert!(right_arrow[0][0][0] > right_arrow[0][2][0]);
    assert!(left_arrow[0][0][0] < left_arrow[0][1][0]);
    assert!(left_arrow[0][0][0] < left_arrow[0][2][0]);
    assert!(line_text_xs(&draw_list, &lines[1], "→").is_empty());
    assert!(line_text_xs(&draw_list, &lines[2], "←").is_empty());
    assert!(fill_triangles_for_line(&draw_list, &lines[3]).is_empty());
    assert_eq!(line_text_xs(&draw_list, &lines[3], "→").len(), 1);
}
```

- [ ] **Step 2: 运行测试并确认因缺少几何箭头而失败**

Run:

```bash
cargo test -p textora-markdown --lib -- render_horizontal_connector_arrows_share_box_line_center
```

Expected: FAIL；右箭头行或左箭头行的三角形数量为 `0`，不是编译错误或 fixture 检测失败。

- [ ] **Step 3: 实现最小连续箭头识别与绘制**

在 `render.rs` 增加私有方向枚举，并根据相邻网格格识别连接箭头：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HorizontalArrowDirection {
    Left,
    Right,
}

fn horizontal_arrow_direction(
    row: &AsciiDiagramRow,
    cell_index: usize,
) -> Option<HorizontalArrowDirection> {
    let cell = row.cells.get(cell_index)?;
    match cell.text.as_str() {
        "→" if cell_index
            .checked_sub(1)
            .and_then(|index| row.cells.get(index))
            .is_some_and(|neighbor| neighbor.text == "─") =>
        {
            Some(HorizontalArrowDirection::Right)
        }
        "←" if row.cells.get(cell_index + 1).is_some_and(|neighbor| neighbor.text == "─") => {
            Some(HorizontalArrowDirection::Left)
        }
        _ => None,
    }
}

fn draw_horizontal_arrow(
    direction: HorizontalArrowDirection,
    cell_x: f32,
    line_top: f32,
    cell_width: f32,
    line_height: f32,
    font_size: f32,
    color: [f32; 4],
    draw_list: &mut DrawList,
) {
    const TIP_INSET_RATIO: f32 = 0.1;
    const HEAD_LENGTH_RATIO: f32 = 0.45;
    const HEAD_HALF_HEIGHT_FONT_RATIO: f32 = 0.28;
    const MAXIMUM_HEAD_HALF_LINE_RATIO: f32 = 0.4;

    let thickness = (font_size * 0.08).clamp(1.0, 2.0);
    let half_thickness = thickness * 0.5;
    let center_y = line_top + line_height * 0.5;
    let head_half_height = (font_size * HEAD_HALF_HEIGHT_FONT_RATIO)
        .min(line_height * MAXIMUM_HEAD_HALF_LINE_RATIO);
    let tip_inset = cell_width * TIP_INSET_RATIO;
    let head_length = cell_width * HEAD_LENGTH_RATIO;
    let (tip_x, base_x, shaft_left, shaft_right) = match direction {
        HorizontalArrowDirection::Left => {
            let tip_x = cell_x + tip_inset;
            let base_x = tip_x + head_length;
            (tip_x, base_x, base_x - half_thickness, cell_x + cell_width)
        }
        HorizontalArrowDirection::Right => {
            let tip_x = cell_x + cell_width - tip_inset;
            let base_x = tip_x - head_length;
            (tip_x, base_x, cell_x, base_x + half_thickness)
        }
    };

    draw_list.fill(
        Rect::new(
            shaft_left,
            center_y - half_thickness,
            shaft_right - shaft_left,
            thickness,
        ),
        color,
    );
    draw_list.fill_triangle(
        [tip_x, center_y],
        [base_x, center_y - head_half_height],
        [base_x, center_y + head_half_height],
        color,
    );
}
```

把 `render_ascii_diagram_row` 的循环改为 `for (cell_index, cell) in row.cells.iter().enumerate()`，并在普通文本 shaping 前加入：

```rust
if let Some(direction) = horizontal_arrow_direction(row, cell_index) {
    draw_horizontal_arrow(
        direction,
        cell_x,
        line_top,
        allocated_width,
        line.rect.h,
        line.font_size,
        color,
        dl,
    );
    cell_byte_start = cell_byte_end;
    continue;
}
```

- [ ] **Step 4: 运行定向测试并确认通过**

Run:

```bash
cargo test -p textora-markdown --lib -- render_horizontal_connector_arrows_share_box_line_center
```

Expected: PASS；连续左右箭头均为几何绘制，独立箭头保持文本绘制。

- [ ] **Step 5: 运行格式化、crate 测试与编译检查**

Run:

```bash
cargo fmt --check
cargo test -p textora-markdown
cargo check -p textora-markdown
```

Expected: 三条命令全部成功，无新增 warning。

- [ ] **Step 6: 提交实现**

```bash
git add crates/markdown/src/render.rs \
  docs/superpowers/specs/2026-07-17-ascii-diagram-horizontal-arrow-alignment-design.md \
  docs/superpowers/plans/2026-07-17-ascii-diagram-horizontal-arrow-alignment.md
git commit -m "fix(markdown): align horizontal connector arrows"
```
