# Markdown ASCII 框线图固定网格渲染实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development`（推荐）或 `executing-plans` 逐任务实施。步骤使用复选框跟踪。

**Goal:** 自动识别 Markdown fenced code block 中的 Unicode 框线图，并在非编辑态以固定列网格和几何框线渲染，保证 CJK 混排时边框严格对齐。

**Architecture:** 在 `markdown::layout` 新增纯数据 ASCII 图模型与检测函数；布局阶段仅为非活动代码块附加该模型。渲染阶段优先按模型的逻辑列定位文字，并把常用轻线框字符转换为 `DrawList` 几何线段；其他代码块路径、源文本和编辑映射保持原样。

**Tech Stack:** Rust、`unicode-segmentation`、现有 `shaping::Shaper`、`ui::core::paint::DrawList`、`textora-markdown`。

## 全局约束

- 只处理 fenced code block；不修改 Markdown 源文本、复制结果或语法高亮。
- 自动检测必须同时满足：至少两行非空文本、至少一个角点、至少六个框线字符且分布于两行以上。
- ASCII 与常用框线字符占 1 列；CJK/全角 grapheme cluster 占 2 列；组合标记不额外占列。
- 首版仅在非活动代码块启用；活动代码块继续现有文本渲染，保障光标、选择与 IME。
- `ui` 不得依赖 Markdown 领域类型；网格模型只存在于 `crates/markdown`。
- 每个任务先写失败测试，运行确认失败，再最小实现、复测并提交。

## 文件结构

| 文件 | 职责 |
| --- | --- |
| `crates/markdown/src/layout/ascii_diagram.rs` | 纯检测、grapheme 到逻辑列的转换，以及轻线框字符的连接方向映射。 |
| `crates/markdown/src/layout/mod.rs` | 注册内部模块并向 layout、renderer 重导出模型。 |
| `crates/markdown/src/layout/types.rs` | 在 `CodeBlock` 布局输出中承载可选 `AsciiDiagram`，不改变 source projection。 |
| `crates/markdown/src/layout/block.rs` | 只为非活动代码块调用检测函数，并附加网格模型。 |
| `crates/markdown/src/render.rs` | 测量代码单元宽度、按逻辑列定位文字、几何绘制常用轻线框。 |

---

### Task 1: 建立纯 ASCII 图检测与网格模型

**Files:**

- Create: `crates/markdown/src/layout/ascii_diagram.rs`
- Modify: `crates/markdown/src/layout/mod.rs`
- Test: `crates/markdown/src/layout/ascii_diagram.rs`

**Interfaces:**

- Produces: `pub(crate) fn detect_ascii_diagram(lines: &[String]) -> Option<AsciiDiagram>`。
- Produces: `AsciiDiagram { rows, column_count }`、`AsciiDiagramRow { cells, column_count }`、`AsciiDiagramCell { text, column, column_width, box_connections }`。
- Produces: `BoxConnections { left, right, up, down }`，供 renderer 将常用轻线框字符转换为几何线段。
- Consumes: `super::context::is_cjk_or_fullwidth` 和 `unicode_segmentation::UnicodeSegmentation`。

- [ ] **Step 1: 写入检测与列宽的失败测试**

在新文件中先建立测试模块；此时尚未定义目标函数，编译应失败。

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn lines(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn detects_light_box_diagram_with_cjk_label() {
        let diagram = detect_ascii_diagram(&lines(&[
            "┌────┐",
            "│中文│",
            "└────┘",
        ]))
        .expect("a multi-line light box must be detected");

        assert_eq!(diagram.rows.len(), 3);
        assert_eq!(diagram.column_count, 6);
        assert!(diagram.rows.iter().all(|row| row.column_count == 6));
        assert_eq!(diagram.rows[1].cells[1].column_width, 2);
    }

    #[test]
    fn rejects_normal_code_and_single_box_character() {
        assert!(detect_ascii_diagram(&lines(&["let result = value + 1;", "println!(\"{result}\");"])).is_none());
        assert!(detect_ascii_diagram(&lines(&["┌ value", "plain text"])).is_none());
    }

    #[test]
    fn maps_all_supported_light_box_connections() {
        assert_eq!(box_connections("─"), Some(BoxConnections::LEFT_RIGHT));
        assert_eq!(box_connections("│"), Some(BoxConnections::UP_DOWN));
        assert_eq!(box_connections("┌"), Some(BoxConnections::RIGHT_DOWN));
        assert_eq!(box_connections("┼"), Some(BoxConnections::ALL));
    }
}
```

- [ ] **Step 2: 运行测试并确认失败**

Run: `cargo test -p textora-markdown layout::ascii_diagram::tests --no-run`

Expected: FAIL，提示 `detect_ascii_diagram`、`BoxConnections` 和 `box_connections` 尚未定义。

- [ ] **Step 3: 实现纯模型、字符分类和检测函数**

在 `crates/markdown/src/layout/ascii_diagram.rs` 写入以下接口与实现。框线字符计数只统计 `box_connections()` 返回 `Some` 的轻线字符，满足“至少一个角点、至少六个字符、至少两行”后才返回布局。

```rust
use unicode_segmentation::UnicodeSegmentation;

use super::context::is_cjk_or_fullwidth;

const MINIMUM_BOX_DRAWING_CHARACTERS: usize = 6;
const MINIMUM_BOX_DRAWING_LINES: usize = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BoxConnections {
    pub(crate) left: bool,
    pub(crate) right: bool,
    pub(crate) up: bool,
    pub(crate) down: bool,
}

impl BoxConnections {
    pub(crate) const LEFT_RIGHT: Self = Self { left: true, right: true, up: false, down: false };
    pub(crate) const UP_DOWN: Self = Self { left: false, right: false, up: true, down: true };
    pub(crate) const RIGHT_DOWN: Self = Self { left: false, right: true, up: false, down: true };
    pub(crate) const ALL: Self = Self { left: true, right: true, up: true, down: true };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AsciiDiagramCell {
    pub(crate) text: String,
    pub(crate) column: usize,
    pub(crate) column_width: usize,
    pub(crate) box_connections: Option<BoxConnections>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AsciiDiagramRow {
    pub(crate) cells: Vec<AsciiDiagramCell>,
    pub(crate) column_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AsciiDiagram {
    pub(crate) rows: Vec<AsciiDiagramRow>,
    pub(crate) column_count: usize,
}

pub(crate) fn box_connections(text: &str) -> Option<BoxConnections> {
    match text {
        "─" => Some(BoxConnections::LEFT_RIGHT),
        "│" => Some(BoxConnections::UP_DOWN),
        "┌" => Some(BoxConnections::RIGHT_DOWN),
        "┐" => Some(BoxConnections { left: true, down: true, ..BoxConnections::default() }),
        "└" => Some(BoxConnections { right: true, up: true, ..BoxConnections::default() }),
        "┘" => Some(BoxConnections { left: true, up: true, ..BoxConnections::default() }),
        "├" => Some(BoxConnections { right: true, up: true, down: true, left: false }),
        "┤" => Some(BoxConnections { left: true, up: true, down: true, right: false }),
        "┬" => Some(BoxConnections { left: true, right: true, down: true, up: false }),
        "┴" => Some(BoxConnections { left: true, right: true, up: true, down: false }),
        "┼" => Some(BoxConnections::ALL),
        _ => None,
    }
}

pub(crate) fn detect_ascii_diagram(lines: &[String]) -> Option<AsciiDiagram> {
    if lines.iter().filter(|line| !line.is_empty()).count() < 2 {
        return None;
    }

    let mut box_character_count = 0usize;
    let mut box_line_count = 0usize;
    let mut has_corner = false;
    let mut rows = Vec::with_capacity(lines.len());

    for line in lines {
        let (row, row_box_count, row_has_corner) = grid_row(line);
        box_character_count += row_box_count;
        box_line_count += usize::from(row_box_count > 0);
        has_corner |= row_has_corner;
        rows.push(row);
    }

    if !has_corner
        || box_character_count < MINIMUM_BOX_DRAWING_CHARACTERS
        || box_line_count < MINIMUM_BOX_DRAWING_LINES
    {
        return None;
    }

    let column_count = rows.iter().map(|row| row.column_count).max().unwrap_or(0);
    Some(AsciiDiagram { rows, column_count })
}

fn grid_row(line: &str) -> (AsciiDiagramRow, usize, bool) {
    let mut cells = Vec::new();
    let mut column = 0usize;
    let mut box_count = 0usize;
    let mut has_corner = false;

    for grapheme in UnicodeSegmentation::graphemes(line, true) {
        let connections = box_connections(grapheme);
        if connections.is_some() {
            box_count += 1;
            has_corner |= matches!(grapheme, "┌" | "┐" | "└" | "┘");
        }
        let column_width = grapheme_column_width(grapheme);
        cells.push(AsciiDiagramCell {
            text: grapheme.to_owned(),
            column,
            column_width,
            box_connections: connections,
        });
        column += column_width;
    }

    (AsciiDiagramRow { cells, column_count: column }, box_count, has_corner)
}

fn grapheme_column_width(grapheme: &str) -> usize {
    grapheme.chars().next().map_or(0, |character| {
        if is_cjk_or_fullwidth(character) { 2 } else { 1 }
    })
}
```

在 `crates/markdown/src/layout/mod.rs` 中注册模块与 crate 内重导出：

```rust
pub(crate) mod ascii_diagram;
pub(crate) use ascii_diagram::{AsciiDiagram, AsciiDiagramRow, BoxConnections, detect_ascii_diagram};
```

- [ ] **Step 4: 运行模块测试并确认通过**

Run: `cargo test -p textora-markdown layout::ascii_diagram::tests`

Expected: PASS，3 个测试通过。

- [ ] **Step 5: 提交纯模型**

```bash
git add crates/markdown/src/layout/ascii_diagram.rs crates/markdown/src/layout/mod.rs
git commit -m "feat(markdown): detect ascii box diagrams"
```

### Task 2: 将网格模型附加到非活动代码块布局

**Files:**

- Modify: `crates/markdown/src/layout/types.rs:895-901`
- Modify: `crates/markdown/src/layout/block.rs:103-190`
- Test: `crates/markdown/src/layout/block.rs`

**Interfaces:**

- Consumes: `detect_ascii_diagram(&lines)`，仅在 `active == false` 时调用。
- Produces: `LaidOutBlockKind::CodeBlock { lines, language, ascii_diagram }`，其中 `ascii_diagram: Option<AsciiDiagram>`。
- Compatibility: 所有既有模式匹配继续使用 `..`，`LazyLayout` 的 source projection 和 flattened lines 不变。

- [ ] **Step 1: 写入布局与编辑态回退的失败测试**

在 `crates/markdown/src/layout/block.rs` 的测试模块中新增以下 fixture 与测试：

```rust
const ASCII_DIAGRAM_SOURCE: &str = "```\n┌────┐\n│中文│\n└────┘\n```";

#[test]
fn layout_marks_non_active_box_diagram_code_block() {
    let laid_out = layout_doc_with_width(ASCII_DIAGRAM_SOURCE, 400.0);
    let block = laid_out.blocks.first().expect("fixture has one code block");
    let LaidOutBlockKind::CodeBlock { ascii_diagram, .. } = &block.kind else {
        panic!("fixture must produce a code block");
    };
    assert_eq!(ascii_diagram.as_ref().map(|diagram| diagram.column_count), Some(6));
}

#[test]
fn active_box_diagram_code_block_keeps_normal_layout_path() {
    let cursor_byte = ASCII_DIAGRAM_SOURCE.find("中文").expect("fixture has CJK label");
    let layout = layout_with_cursor_and_width(ASCII_DIAGRAM_SOURCE, cursor_byte, 400.0);
    let block = layout.laid_out[0].as_ref().expect("visible code block must materialize");
    let LaidOutBlockKind::CodeBlock { ascii_diagram, .. } = &block.kind else {
        panic!("fixture must produce a code block");
    };
    assert!(ascii_diagram.is_none(), "active code blocks must keep the existing path");
}
```

- [ ] **Step 2: 运行测试并确认失败**

Run: `cargo test -p textora-markdown layout::block::tests::layout_marks_non_active_box_diagram_code_block`

Expected: FAIL，提示 `CodeBlock` 不存在 `ascii_diagram` 字段。

- [ ] **Step 3: 扩展布局输出并只为非活动块构造网格**

在 `crates/markdown/src/layout/types.rs` 引入并增加字段：

```rust
use super::ascii_diagram::AsciiDiagram;

// LaidOutBlockKind 中
CodeBlock {
    lines: Vec<LaidOutLine>,
    language: Option<String>,
    ascii_diagram: Option<AsciiDiagram>,
},
```

在 `crates/markdown/src/layout/block.rs` 的 `BlockKind::CodeBlock` 分支中，于 `lines` 构造完成之后、创建 `laid_out_lines` 之前计算：

```rust
let ascii_diagram = if active { None } else { super::ascii_diagram::detect_ascii_diagram(&lines) };
```

并替换 block 构造为：

```rust
LaidOutBlockKind::CodeBlock {
    lines: laid_out_lines,
    language: language.clone(),
    ascii_diagram,
}
```

不要改动 `LaidOutLine`、source projection、code block 高度或语法高亮构造逻辑。

- [ ] **Step 4: 运行布局测试并确认通过**

```bash
cargo test -p textora-markdown layout_marks_non_active_box_diagram_code_block
cargo test -p textora-markdown active_box_diagram_code_block_keeps_normal_layout_path
```

Expected: 两条命令均 PASS。

- [ ] **Step 5: 运行代码块和 LazyLayout 回归测试**

Run: `cargo test -p textora-markdown code_block`

Expected: PASS，既有代码块及 source projection 测试不回归。

- [ ] **Step 6: 提交布局集成**

```bash
git add crates/markdown/src/layout/types.rs crates/markdown/src/layout/block.rs
git commit -m "feat(markdown): attach grid layout to ascii diagrams"
```

### Task 3: 按网格定位文字并绘制常用轻线框

**Files:**

- Modify: `crates/markdown/src/render.rs:1-18`
- Modify: `crates/markdown/src/render.rs:134-166`
- Modify: `crates/markdown/src/render.rs`（新增 ASCII 图渲染辅助函数与测试）
- Test: `crates/markdown/src/render.rs`

**Interfaces:**

- Consumes: `AsciiDiagramRow.cells` 的 `column`、`column_width` 与 `box_connections`。
- Consumes: `Shaper::col_width()`，在临时设置代码字体、字号后测量单元格宽度并恢复状态。
- Produces: 同一逻辑列的 `│` 对应相同 x 坐标的 `DrawCmd::FillRect`；普通文字对应独立 `DrawCmd::TextLayout`。
- Fallback: 无 shaper、缺少 diagram 行、或未映射字符时走安全的既有文本渲染，不得 panic。

- [ ] **Step 1: 写入 renderer 的失败测试**

在 `crates/markdown/src/render.rs` 测试模块中添加：

```rust
#[test]
fn render_ascii_diagram_places_vertical_borders_on_one_grid_column() {
    let dl = build_and_render("```\n┌────┐\n│中文│\n│内容│\n└────┘\n```");
    let mut vertical_xs: Vec<f32> = dl
        .cmds
        .iter()
        .filter_map(|command| match command {
            DrawCmd::FillRect { rect, .. } if rect.w <= 2.0 && rect.h > 8.0 => Some(rect.x),
            _ => None,
        })
        .collect();

    assert!(vertical_xs.len() >= 4, "two borders across two rows must emit vertical segments");
    vertical_xs.sort_by(f32::total_cmp);
    assert!(
        vertical_xs.windows(2).any(|pair| (pair[0] - pair[1]).abs() < 0.01),
        "the same vertical border must use one x coordinate across rows: {vertical_xs:?}"
    );
}

#[test]
fn render_normal_code_block_keeps_single_text_line_path() {
    let dl = build_and_render("```\nlet value = 1;\n```");
    let text_count = dl
        .cmds
        .iter()
        .filter(|command| matches!(command, DrawCmd::TextLayout { layout, .. } if layout.text == "let value = 1;"))
        .count();
    assert_eq!(text_count, 1, "normal code must not be split into grid cells");
}

#[test]
fn render_active_ascii_diagram_keeps_text_path() {
    let source = "```\n┌────┐\n│中文│\n└────┘\n```";
    let cursor = source.find("中文").expect("fixture has CJK label");
    let (dl, _) = build_and_render_editing(source, cursor);
    assert!(dl.cmds.iter().any(
        |command| matches!(command, DrawCmd::TextLayout { layout, .. } if layout.text.contains("中文"))
    ));
}
```

- [ ] **Step 2: 运行 renderer 测试并确认失败**

Run: `cargo test -p textora-markdown render_ascii_diagram_places_vertical_borders_on_one_grid_column`

Expected: FAIL，尚未存在用于框线的细长 `FillRect`。

- [ ] **Step 3: 实现代码单元宽度测量与框线绘制**

在 `render.rs` 顶部引入：

```rust
use std::sync::Arc;

use crate::layout::{AsciiDiagram, AsciiDiagramRow, BoxConnections};
use ui::core::text_layout::UiTextLayout;
```

增加不泄漏 shaper 状态的测量函数：

```rust
fn code_cell_width(shaper: &mut shaping::Shaper, font_size: f32, family: Option<&str>) -> f32 {
    let old_size = shaper.font_size();
    let old_family = shaper.font_family().map(str::to_owned);
    shaper.set_font_size(font_size);
    shaper.set_font_family(family);
    let width = shaper.col_width();
    shaper.set_font_size(old_size);
    shaper.set_font_family(old_family.as_deref());
    width
}
```

增加 `draw_box_connections`。线宽采用 `font_size * 0.08`，限制在 `[1.0, 2.0]`；水平线从单元格左/右边界连接至中心，垂直线从行上/下边界连接至中心：

```rust
fn draw_box_connections(
    connections: BoxConnections,
    cell_x: f32,
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
        dl.fill(Rect::new(cell_x, center_y - half, cell_width * 0.5 + half, thickness), color);
    }
    if connections.right {
        dl.fill(Rect::new(center_x - half, center_y - half, cell_width * 0.5 + half, thickness), color);
    }
    if connections.up {
        dl.fill(Rect::new(center_x - half, line_top, thickness, line_height * 0.5 + half), color);
    }
    if connections.down {
        dl.fill(Rect::new(center_x - half, center_y - half, thickness, line_height * 0.5 + half), color);
    }
}
```

增加 `render_ascii_diagram_row`：跳过全空白单元格；对 `box_connections` 调用上面的几何函数；其他单元格用 `UiTextLayout::new` shape 后，根据 `column_width * cell_width` 在分配区域内居中，并通过 `dl.text_layout(Arc::new(layout), x, baseline, color)` 发出命令。不得使用整行 `render_line_with_offset`，因为它会重新累加自然字形 advance。

- [ ] **Step 4: 在 CodeBlock 渲染分支接入网格路径**

将匹配臂改为取得 `ascii_diagram`。仍先绘制既有背景、边框和 clip；在 clip 内：当 `ascii_diagram` 与 `shaper` 都存在、且 `diagram.rows.len() == lines.len()` 时，逐行调用 `render_ascii_diagram_row`；否则保持当前逐行 `render_line_with_offset` 循环。

核心分支应为：

```rust
if let (Some(diagram), Some(shaper)) = (ascii_diagram.as_ref(), shaper.as_deref_mut())
    && diagram.rows.len() == lines.len()
{
    let cell_width = code_cell_width(shaper, style.code_font_size, style.code_font_family.as_deref());
    for (line, row) in lines.iter().zip(&diagram.rows) {
        if line.rect.y + line.rect.h < scroll_y {
            continue;
        }
        if line.rect.y > scroll_y + viewport_h {
            break;
        }
        render_ascii_diagram_row(line, row, cell_width, style, dl, scroll_y, ox, oy, shaper);
    }
} else {
    // 保留原有 render_line_with_offset 循环，内容不变。
}
```

- [ ] **Step 5: 运行 renderer 测试并确认通过**

```bash
cargo test -p textora-markdown render_ascii_diagram_places_vertical_borders_on_one_grid_column
cargo test -p textora-markdown render_normal_code_block_keeps_single_text_line_path
cargo test -p textora-markdown render_active_ascii_diagram_keeps_text_path
```

Expected: 三条命令均 PASS；图表产生几何边框，普通/活动代码块保持既有文本路径。

- [ ] **Step 6: 提交渲染实现**

```bash
git add crates/markdown/src/render.rs
git commit -m "feat(markdown): render ascii diagrams on a fixed grid"
```

### Task 4: 全量验证与视觉验收

**Files:**

- Modify: `docs/specs/2026-07-15-ascii-diagram-grid-rendering-design.md`（仅在验收结论需要记录时）
- Test: `crates/markdown/src/layout/ascii_diagram.rs`
- Test: `crates/markdown/src/layout/block.rs`
- Test: `crates/markdown/src/render.rs`

**Interfaces:**

- Verifies: 纯模型、布局集成、渲染路径以及既有 Markdown 回归测试。

- [ ] **Step 1: 格式化并运行 crate 测试**

```bash
cargo fmt --check
cargo test -p textora-markdown
```

Expected: 两条命令均成功；无格式差异，`textora-markdown` 全部测试通过。

- [ ] **Step 2: 运行项目级验证**

```bash
./scripts/verify.sh
```

Expected: 项目验证脚本成功完成。若脚本失败，按失败输出先补充最小复现测试，再修复根因；不要用降级或跳过测试掩盖失败。

- [ ] **Step 3: 执行手动视觉验收**

新建或打开包含以下代码块的 Markdown 文档，确认预览中左右边框、分隔线和交叉点对齐；将光标置于 `客户端` 内，确认该块回退到正常编辑显示；失焦后恢复网格图：

```text
┌──────── WPS 客户端 ────────┐
│ · 本地日志（30天滚动）     │
│ · 焦点渲染                 │
└────────────────────────────┘
```

- [ ] **Step 4: 提交验证产生的必要调整**

```bash
git add crates/markdown/src docs/specs/2026-07-15-ascii-diagram-grid-rendering-design.md
git commit -m "test(markdown): verify ascii diagram grid rendering"
```

只在本任务实际产生未提交改动时执行该提交；若无改动，不创建空提交。
