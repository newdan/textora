# ASCII Diagram Cumulative Border Alignment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Markdown ASCII 图中的逻辑边界允许缺口，但所有已确认竖边都在从整行最左侧累计后的同一网格列，并让前方对齐增量自动传递给箭头、并排矩形及后续内容。

**Architecture:** 拓扑阶段只识别字符属于哪个矩形边；坐标阶段把一次右移表示为在该边之前插入虚拟列，并将增量传播给本行后缀。逻辑竖边按从左到右求解，每条轨道使用已经包含前序偏移的当前渲染列确定目标。累计位移与右角点自身产生的横线延长量分开存储。

**Tech Stack:** Rust 2024、`unicode-segmentation`、`unicode-width`、项目内 Markdown 布局与 `DrawList` 渲染测试。

## Global Constraints

- 产品名是 `textora`，Markdown 包名是 `textora-markdown`。
- 不修改 Markdown 源内容，不创建源图中缺失的 `AsciiDiagramCell`。
- 最终列必须从行首累计；禁止用服务端局部矩形跨度反推其绝对左边界。
- 只插入虚拟列并右移当前边及其行后缀；禁止左移已有内容。
- 固定一列跨度上限已废弃；跨度只能作为拓扑归属证据。
- `Missing` 不阻止同轨道其他行对齐；`Ambiguous` 不得强行认领字符。
- 嵌套框、并排框、时间轴和连接箭头必须保持独立归属。
- 继承前缀位移的箱线字符不得重复延长横线；只有本次插列位置上的左向连接可以填补新增间隔。
- Rust 代码禁止新增无说明的 `.unwrap()`；确定不失败的位置使用带原因的 `.expect(...)`。
- 每次提交前运行 `cargo fmt --all -- --check` 和 `cargo check -p textora-markdown`。
- 最终运行 `./scripts/verify.sh`。

## File Structure

- Modify: `crates/markdown/src/layout/ascii_diagram.rs`
  - 保存源列、累计渲染位移和局部横线延长列数。
  - 提供在指定单元格前插入虚拟列并传播到行后缀的原语。
  - 识别矩形逻辑边并按从左到右应用轨道。
  - 包含最小行为、嵌套保护及完整 WPS 输入的布局测试。
- Modify: `crates/markdown/src/render.rs`
  - 使用布局侧显式提供的局部横线延长量。
  - 包含并排框后缀传播及环形缓冲区像素共线测试。

---

### Task 1: 已完成的断续右边基础

**Status:** Complete，提交 `3dcf5f27`、`e657f419` 已通过任务审查。

**保留行为：**

- `RightEdgeAssignment` 显式区分 `Missing`、`Assigned`、`Ambiguous`。
- 缺少右边的行不再直接否决整个矩形。
- 左边缺失时不能认领无关右边。

**后续修订：** Task 3 将替换“左边必须精确等于角点源列”和独立绝对列吸附；Task 1 的提交不回退，但其局部匹配实现不再作为最终架构约束。

---

### Task 2: 建立累计虚拟列与行后缀传播原语

**Files:**
- Modify: `crates/markdown/src/layout/ascii_diagram.rs:33-55,217-240,607-625`
- Modify: `crates/markdown/src/render.rs:561-600`
- Test: `crates/markdown/src/layout/ascii_diagram.rs` 同文件测试模块
- Test: `crates/markdown/src/render.rs` 同文件测试模块

**Interfaces:**

- Produces: `AsciiDiagramCell::render_column() -> usize` 保持调用接口不变。
- Produces: `AsciiDiagramCell::left_extension_columns() -> usize`。
- Produces: `align_row_suffix_to(row: &mut AsciiDiagramRow, cell_index: usize, target_column: usize)`。
- Consumes: 渲染层继续按 `render_column` 计算 `cell_x`，但横线延长改用 `left_extension_columns`。

- [ ] **Step 1: 写行后缀传播的失败测试**

在 `ascii_diagram.rs` 测试模块加入：

```rust
#[test]
fn aligning_one_edge_shifts_every_following_cell() {
    let (mut row, _, _) = grid_row("┌─┐  ┌─┐");
    let first_right_index = row
        .cells
        .iter()
        .position(|cell| cell.column == 2)
        .expect("fixture contains the first right corner");

    align_row_suffix_to(&mut row, first_right_index, 4);

    assert_eq!(cell_at_source_column(&row, 2).render_column(), 4);
    assert_eq!(cell_at_source_column(&row, 5).render_column(), 7);
    assert_eq!(cell_at_source_column(&row, 7).render_column(), 9);
}
```

- [ ] **Step 2: 运行测试并确认 RED**

Run:

```bash
cargo test -p textora-markdown aligning_one_edge_shifts_every_following_cell -- --exact
```

Expected: 编译失败或断言失败，因为当前仅有单单元格 `align_right_edge_to`，不存在行后缀传播。

- [ ] **Step 3: 写“累计位移与局部横线延长分离”的失败测试**

```rust
#[test]
fn inherited_shift_does_not_extend_later_horizontal_edges() {
    let (mut row, _, _) = grid_row("┌─┐  ┌─┐");
    let first_right_index = row
        .cells
        .iter()
        .position(|cell| cell.column == 2)
        .expect("fixture contains the first right corner");

    align_row_suffix_to(&mut row, first_right_index, 4);

    assert_eq!(cell_at_source_column(&row, 2).left_extension_columns(), 2);
    assert_eq!(cell_at_source_column(&row, 5).left_extension_columns(), 0);
    assert_eq!(cell_at_source_column(&row, 7).left_extension_columns(), 0);
}
```

Run:

```bash
cargo test -p textora-markdown inherited_shift_does_not_extend_later_horizontal_edges -- --exact
```

Expected: RED；当前渲染把 `render_column - source_column` 当成每个箱线字符自己的延长量，无法区分继承位移。

- [ ] **Step 4: 实现最小累计表示**

将 `AsciiDiagramCell` 的 `aligned_column` 替换为两个含义独立的字段：

```rust
pub(crate) struct AsciiDiagramCell {
    pub(crate) text: String,
    pub(crate) column: usize,
    pub(crate) column_width: usize,
    pub(crate) box_connections: Option<BoxConnections>,
    render_column_shift: usize,
    left_extension_columns: usize,
}

impl AsciiDiagramCell {
    pub(crate) fn render_column(&self) -> usize {
        self.column + self.render_column_shift
    }

    pub(crate) fn left_extension_columns(&self) -> usize {
        self.left_extension_columns
    }

    fn shift_render_column_by(&mut self, column_count: usize) {
        self.render_column_shift += column_count;
    }
}
```

`grid_row` 初始化两个字段为 `0`。新增：

```rust
fn align_row_suffix_to(
    row: &mut AsciiDiagramRow,
    cell_index: usize,
    target_column: usize,
) {
    let current_column = row.cells[cell_index].render_column();
    let shift = target_column.saturating_sub(current_column);
    if shift == 0 {
        return;
    }

    if row.cells[cell_index]
        .box_connections
        .is_some_and(|connections| connections.left)
    {
        row.cells[cell_index].left_extension_columns += shift;
    }
    for cell in &mut row.cells[cell_index..] {
        cell.shift_render_column_by(shift);
    }
}
```

Task 2 保留 `AsciiDiagramCell::align_right_edge_to(target_column)` 供旧矩形算法继续执行单单元格吸附，并用新字段表达该单元格的位移和局部横线延长。`align_row_suffix_to(...)` 在本任务只由新增原语测试调用。

原因：旧算法预先按源列计算所有矩形目标；如果在 Task 2 直接传播内框增量，后续外框会继承位移，但外框目标尚未基于新当前列重算，导致中间提交不共线。Task 3 将在同一次变更中切换生产调用并让右侧轨道重新计算目标。

- [ ] **Step 5: 修改渲染层只使用局部延长量**

将：

```rust
let left_extension_width =
    render_column.saturating_sub(cell.column) as f32 * cell_width;
```

替换为：

```rust
let left_extension_width = cell.left_extension_columns() as f32 * cell_width;
```

- [ ] **Step 6: 运行定向与模块测试并确认 GREEN**

```bash
cargo test -p textora-markdown aligning_one_edge_shifts_every_following_cell -- --exact
cargo test -p textora-markdown inherited_shift_does_not_extend_later_horizontal_edges -- --exact
cargo test -p textora-markdown layout::ascii_diagram::tests -- --nocapture
cargo test -p textora-markdown render_snapped_rectangle -- --nocapture
```

Expected: 全部 PASS；既有右角点横线延长测试不得回退。

- [ ] **Step 7: 格式化、编译并提交**

```bash
cargo fmt --all -- --check
cargo check -p textora-markdown
git diff --check
git add crates/markdown/src/layout/ascii_diagram.rs crates/markdown/src/render.rs
git commit -m "refactor(markdown): propagate diagram column shifts"
```

---

### Task 3: 按从左到右的逻辑轨道累计对齐完整架构图

**Files:**
- Modify: `crates/markdown/src/layout/ascii_diagram.rs:250-625`
- Test: `crates/markdown/src/layout/ascii_diagram.rs` 同文件测试模块

**Interfaces:**

- Produces: `BorderSide::{Left, Right}`。
- Produces: `BorderMember { row_index, cell_index }`。
- Produces: `VerticalBorderTrack { rectangle_index, side, members }`。
- Produces: `vertical_border_tracks(rows: &[AsciiDiagramRow]) -> Vec<VerticalBorderTrack>`。
- Produces: `align_vertical_border_tracks(rows: &mut [AsciiDiagramRow])`。
- Consumes: Task 2 的 `align_row_suffix_to(...)`。

- [ ] **Step 1: 写两级累计的最小失败测试**

```rust
#[test]
fn later_track_uses_columns_shifted_by_the_previous_track() {
    let (first_row, _, _) = grid_row("│  │  │  │");
    let (mut second_row, _, _) = grid_row("│ │  │ │");

    align_row_suffix_to(&mut second_row, 2, 3);
    assert_eq!(cell_at_source_column(&second_row, 5).render_column(), 6);

    align_row_suffix_to(&mut second_row, 5, 7);
    assert_eq!(cell_at_source_column(&second_row, 7).render_column(), 9);
    assert_eq!(cell_at_source_column(&first_row, 9).render_column(), 9);
}
```

Expected: 在 Task 2 前 RED；Task 2 后 GREEN，锁定“第二轨道读取第一轨道传播后的当前列”。

- [ ] **Step 2: 重写完整架构图测试的断言语义**

保留完整 `wps-focus-mvp-design.md` 65–97 行夹具，但废弃原始绝对目标 `38/82`。对代表性成员取最终列并断言同轨道共线：

```rust
let client_right_columns = [(0, 37), (1, 38), (12, 36), (30, 37)]
    .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
assert!(client_right_columns.windows(2).all(|pair| pair[0] == pair[1]));

let server_left_columns = [(0, 48), (3, 50), (12, 47), (19, 49)]
    .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
assert!(server_left_columns.windows(2).all(|pair| pair[0] == pair[1]));

let server_right_columns = [(0, 80), (3, 82), (12, 79), (30, 79)]
    .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
assert!(server_right_columns.windows(2).all(|pair| pair[0] == pair[1]));
```

同时保留 `missing_outer_side_does_not_claim_an_inner_adjacent_vertical_edge`，期望源列 `10` 不被移动。

- [ ] **Step 3: 运行真实测试并确认 RED**

```bash
cargo test -p textora-markdown aligns_wps_architecture_outer_edges_with_nested_and_missing_segments -- --exact
cargo test -p textora-markdown missing_outer_side_does_not_claim_an_inner_adjacent_vertical_edge -- --exact
```

Expected: 架构图因服务端左边和右边未按前缀累计而 FAIL；嵌套反例必须保持 PASS，若失败不得继续实现。

- [ ] **Step 4: 构建逻辑轨道而不计算绝对坐标**

实现以下数据类型：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BorderSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BorderMember {
    row_index: usize,
    cell_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VerticalBorderTrack {
    rectangle_index: usize,
    side: BorderSide,
    members: Vec<BorderMember>,
}
```

轨道构建必须遵守以下硬约束：

1. 角点成员由 `RectangleCandidate` 直接提供。
2. 每行按字符顺序分配，归属后的成员索引严格递增，保证边界不交叉。
3. 内层矩形先确认自身左右边；外层矩形不能认领已经属于内层的成员。
4. 没有唯一拓扑归属的成员不加入任何轨道。
5. 单独矩形且行内只有唯一左右边对时直接归属，不应用跨度阈值。
6. 嵌套场景中的漂移成员必须得到跨行重复列或重复跨度支持；孤立的 `0..10` 不能替代已有外框 `0..12`。

删除 `MAXIMUM_BORDER_SPAN_DRIFT_COLUMNS`、`rectangle_span_deviation` 和以中点作为硬边界的实现。

最终实现还必须采用以下无阈值、无回溯规则：

- 角点候选用开放栈按“正重叠 → 最近开放行 → 最大重叠唯一”闭合；同层最大重叠并列时不生成候选。
- 低置信行先一次性联合直接列候选与跨度边对候选，再进行唯一性判断；禁止先固定某一种证据。
- 只输出完整候选集内 expected/actual 双向唯一且不与完整集任何候选交叉的成员。
- 纯跨度证据必须左右端同时属于强制集合并互为伙伴；否则两端都不输出。
- 首预期轨道↔首实际边、末预期轨道↔末实际边仅在对应候选得到历史列直接支持，且不存在共享 expected/actual 的其他 direct 候选时作为边界锚点。
- 边界锚点只有一个互相引用的跨度伙伴，且该伙伴不与任何兼容全部已确认边界锚点的候选交叉时，才原子加入伙伴；多个伙伴或仍有可行交叉候选时不得选择。
- 与边界锚点共享 expected/actual 或交叉的候选由边界拓扑事实判为不可行；其余候选不得删除或重新唯一化。边界规则不得扩展为 direct 证据普遍优先。
- 删除候选后不得重新统计唯一性，不递归、回溯或追求最大归属；歧义项保持缺口。
- 若完整候选数为 `K`、跨度伙伴数为 `P`、实际边数为 `C`，交叉否决上界为 `O(K²)`，伙伴验证为 `O(P)`；候选构造还包含 `O(C × Σ|supported_spans[r]|)` 的历史跨度扫描及有序集合操作的对数因子。复杂度说明不得省略候选构造项。

- [ ] **Step 5: 按轨道当前列从左到右累计应用**

```rust
fn align_vertical_border_tracks(rows: &mut [AsciiDiagramRow]) {
    let mut tracks = vertical_border_tracks(rows);
    tracks.sort_by_key(|track| {
        track
            .members
            .iter()
            .map(|member| rows[member.row_index].cells[member.cell_index].render_column())
            .min()
            .unwrap_or(usize::MAX)
    });

    for track in tracks {
        let target_column = track
            .members
            .iter()
            .map(|member| rows[member.row_index].cells[member.cell_index].render_column())
            .max()
            .expect("every vertical border track contains corner members");

        let mut members = track.members;
        members.sort_by_key(|member| (member.row_index, member.cell_index));
        for member in members {
            align_row_suffix_to(&mut rows[member.row_index], member.cell_index, target_column);
        }
    }
}
```

`detect_ascii_diagram` 在构造全部源网格后只调用 `align_vertical_border_tracks(&mut rows)`；目标列必须在处理当前轨道时读取，不能预先缓存原始绝对列。

完成轨道切换后删除仅用于旧矩形算法的单格 `align_right_edge_to(...)`；所有生产对齐统一经 `align_row_suffix_to(...)` 传播。

- [ ] **Step 6: 运行架构、嵌套、并排和滚动窗口测试**

```bash
cargo test -p textora-markdown aligns_wps_architecture_outer_edges_with_nested_and_missing_segments -- --exact
cargo test -p textora-markdown aligns_wps_rolling_window_box_to_its_rightmost_existing_edge -- --exact
cargo test -p textora-markdown direct_last_boundary_anchor_recovers_its_only_reciprocal_span_partner -- --exact
cargo test -p textora-markdown direct_boundary_anchor_does_not_choose_between_multiple_span_partners -- --exact
cargo test -p textora-markdown direct_boundary_anchor_rejects_a_direct_candidate_sharing_its_actual_edge -- --exact
cargo test -p textora-markdown boundary_span_partner_rejects_crossing_candidate_that_remains_viable -- --exact
cargo test -p textora-markdown missing_outer_side_does_not_claim_an_inner_adjacent_vertical_edge -- --exact
cargo test -p textora-markdown ascii_diagram --lib
```

Expected: 全部 PASS；滚动窗口最终右边共线，不再要求最终绝对列等于原始 `61`。

- [ ] **Step 7: 格式化、编译并提交**

```bash
cargo fmt --all -- --check
cargo check -p textora-markdown
git diff --check
git add crates/markdown/src/layout/ascii_diagram.rs
git commit -m "fix(markdown): accumulate diagram border alignment"
```

---

### Task 4: 像素回归、全面验证与人工验收

**Files:**
- Modify: `crates/markdown/src/render.rs` 测试模块
- Verify: `crates/markdown/src/layout/ascii_diagram.rs`
- Reference: `docs/superpowers/specs/2026-07-15-ascii-diagram-discontinuous-border-alignment-design.md`

- [ ] **Step 1: 写三个真实图块的最终像素回归**

所有夹具均经 `render_doc_with_offset_and_ascii_diagrams(...)` 进入最终 `DrawList`：

- 65–97：完整 31 行架构图；顶层轨道累计列固定为 `[0, 38, 51, 84]`，逐行检查所有实际存在的外框轨道，并检查反馈箭头位于客户端右轨道与服务端左轨道之间。反馈行允许缺少客户端外框右边，但服务端左/右边必须落在 `51/84`。
- 116–139：下方滚动窗口 11 行外框全部检查左右像素列，范围必须包含索引 21 的底边角点。
- 220–233：收集每个包含外框字符行的最右竖向绘制坐标，断言：

```rust
assert!(
    right_edge_xs.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 0.01),
    "all existing right-edge segments must share one x coordinate: {right_edge_xs:?}"
);
```

- [ ] **Step 2: 验证新暴露的 RED 后运行最小修正**

```bash
cargo test -p textora-markdown render_wps_ring_buffer_uses_one_right_edge_x_with_discontinuous_source_columns -- --exact
cargo test -p textora-markdown render_wps_architecture_accumulates_all_outer_tracks_from_the_left_edge -- --exact
cargo test -p textora-markdown render_wps_rolling_window_includes_bottom_corners_in_both_outer_tracks -- --exact
```

环形缓冲区在 Task 3 实现上直接 GREEN，作为既有行为的像素特征锁定。架构图首次执行在反馈行 RED：该行因缺少客户端外框右边而被整体保守拒绝，服务端仍停在源列 `46/78`。最小修正只能增加上述有 direct 冲突保护的首末边界锚点，以及经过“仍可行完整候选集”交叉检查的唯一互引跨度伙伴规则；不得降低像素精度、过滤失败行、使用固定漂移阈值或增加真实文件专用分支。

- [ ] **Step 3: 运行 Markdown 包验证并提交测试**

```bash
cargo fmt --all -- --check
cargo check -p textora-markdown
cargo test -p textora-markdown
git diff --check
git add crates/markdown/src/render.rs crates/markdown/src/layout/ascii_diagram.rs
git commit -m "test(markdown): cover cumulative WPS diagram alignment"
```

- [ ] **Step 4: 运行项目全面验证**

```bash
./scripts/verify.sh
```

Expected: 脚本退出码 `0`，包括 workspace fmt、clippy 和测试。

- [ ] **Step 5: 人工复核用户文件**

打开 `/Users/dan/Downloads/wps-focus-mvp-design.md`，检查：

```text
65–97：客户端外框、服务端外框左右边分别共线；箭头及相对布局随前方增量累计。
116–139：时间轴保持独立；下方滚动窗口已有右边共线。
220–233：环形缓冲区已有右边共线；文字无覆盖；横线允许缺口但保持水平。
```

- [ ] **Step 6: 最终代码审查**

审查范围从分支起点到当前 `HEAD`。必须确认：

- 不存在固定漂移阈值或真实文件专用分支；
- 服务端位置没有由局部跨度反推；
- 前缀位移和局部横线延长职责分离；
- 缺口、歧义、嵌套及并排框回归均有覆盖；
- 所有 Critical/Important 问题修复并复审通过。

---

### Task 5: 开放时间轴轨道识别与累计对齐

**Files:**
- Modify: `crates/markdown/src/layout/ascii_diagram.rs`
- Modify: `crates/markdown/src/render.rs` 测试模块
- Reference: `docs/superpowers/specs/2026-07-15-ascii-diagram-discontinuous-border-alignment-design.md`

**Interfaces:**

- Produces: `OpenVerticalTrack`，描述一条由水平主干锚点、相邻 `│` 和可选 `▼` 组成的有序开放轨道。
- Produces: `open_vertical_tracks(rows: &[AsciiDiagramRow]) -> Vec<OpenVerticalTrack>`。
- Produces: `has_open_timeline_structure(rows: &[AsciiDiagramRow]) -> bool`，允许没有矩形角点的强结构时间轴进入固定网格渲染。
- Consumes: 既有 `align_row_suffix_to(...)`，保持只右移和后缀累计传播。

- [ ] **Step 1: 写孤立开放时间轴检测的失败测试**

使用不含 `┌┐└┘` 的十行时间轴夹具调用 `detect_ascii_diagram(...)`，断言返回 `Some`。运行：

```bash
cargo test -p textora-markdown detects_open_timeline_without_rectangle_corners
```

Expected: RED；当前检测被 `has_corner == false` 拒绝。

- [ ] **Step 2: 写开放轨道逻辑列对齐的失败测试**

对同一夹具的主干、空白竖线、中文内容行和箭头行分别收集五个轨道成员的 `render_column()`，全部断言为：

```rust
[2, 20, 37, 47, 57]
```

运行：

```bash
cargo test -p textora-markdown aligns_open_timeline_tracks_with_cjk_content
```

Expected: RED；主干仍为 `[2, 19, 36, 46, 56]`。

- [ ] **Step 3: 最小实现开放结构检测和轨道归属**

扫描连续横线主干中的 `├`、`┼`、`┤` 竖向锚点。仅在相邻行存在数量一致、顺序一致的 `│` 或 `▼` 时建立开放轨道；`▼` 不写入 `box_connections`，继续走文字渲染。将开放轨道成员并入既有从左到右对齐循环，并使用 `align_row_suffix_to(...)` 累计传播。

- [ ] **Step 4: 运行两个布局测试并确认 GREEN**

```bash
cargo test -p textora-markdown detects_open_timeline_without_rectangle_corners
cargo test -p textora-markdown aligns_open_timeline_tracks_with_cjk_content
```

- [ ] **Step 5: 写最终像素回归和歧义保护测试**

在 `render.rs` 对完整 WPS 滚动窗口夹具的时间轴行收集五条竖向轨道的 x 坐标，断言每条轨道跨行共线。另加成员数量不一致夹具，断言无关竖线保持源列。

- [ ] **Step 6: 运行 Markdown 包验证**

```bash
cargo fmt --all -- --check
cargo check -p textora-markdown
cargo test -p textora-markdown
git diff --check
```

- [ ] **Step 7: 运行全面验证**

```bash
./scripts/verify.sh
```
