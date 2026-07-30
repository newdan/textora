# ASCII 图树形分支干扰外框对齐实施计划

> **实施状态：** 已完成（布局提交 `07a26d6d`；渲染回归提交 `74824085`）。

**Goal:** 用带轨道身份的相邻间距证据补全低置信矩形边归属，使 Electron/Go 并排架构图的左外框右边界共线，同时保留树形分支和现有歧义保护。

**Architecture:** `layout::ascii_diagram` 在高置信行中记录相邻逻辑轨道的有向间距，并在低置信行中将命中该间距的两端作为原子候选伙伴加入现有完整候选集。唯一性、非交叉、边界锚点和从左到右累计位移逻辑保持不变；`render` 不新增生产逻辑，只通过最终 `DrawList` 回归验证像素结果。

**Tech Stack:** Rust、`BTreeMap`、`BTreeSet`、`unicode-segmentation`、现有 `AsciiDiagram` 固定网格布局、`ui::core::paint::DrawList`、`textora-markdown`。

**Design:** `docs/plans/2026-07-16-ascii-diagram-tree-branch-border-alignment.md`

## Global Constraints

- 只修改 `crates/markdown/src/layout/ascii_diagram.rs` 和 `crates/markdown/src/render.rs`；不得改动 `ui` 或 `app`。
- 不修改 Markdown 源文本、公开布局 API、编辑态回退路径或字符宽度计算。
- 不按最近距离、固定漂移阈值或样例专用列号推断边界。
- 相邻间距必须绑定确切的有向轨道对，禁止存为无身份的全局距离集合。
- 相邻间距命中的两端必须作为互相引用的原子伙伴共同参与归属，禁止单端成立。
- 候选必须继续经过完整候选集的双向唯一、非交叉和边界锚点验证；删除候选后不得重新唯一化。
- 多个相同间距组合均可成立时保持未归属，不选最近、最左或任意一个组合。
- 不全局过滤 `├`、`└`、`┼` 或 `┤`；开放时间轴逻辑保持不变。
- 遵循 TDD：每个行为先写测试并确认 RED，再写最小生产实现确认 GREEN。
- 若同一问题修改超过两次仍未同时通过真实回归和歧义反例，停止叠加规则并重新审查低置信候选模型。
- 每次提交前必须通过对应 crate 编译；最终重大修改运行 `./scripts/verify.sh`。

## 文件结构

| 文件 | 职责 |
| --- | --- |
| `crates/markdown/src/layout/ascii_diagram.rs` | 保存相邻轨道间距证据，构造原子候选，执行现有低置信归属与累计对齐；承载布局回归和歧义反例。 |
| `crates/markdown/src/render.rs` | 不修改生产渲染；增加真实图表最终像素回归，验证两个外框和连接区的几何关系。 |

---

### Task 1: 用相邻轨道间距补全低置信矩形边归属

**Files:**

- Modify: `crates/markdown/src/layout/ascii_diagram.rs:340-998`
- Test: `crates/markdown/src/layout/ascii_diagram.rs:1152-2179`

**Interfaces:**

- Produces: `AdjacentTrackPair { left_track_index: usize, right_track_index: usize }`，作为有向相邻轨道身份。
- Produces: `type SupportedAdjacentGaps = BTreeMap<AdjacentTrackPair, BTreeSet<usize>>`。
- Produces: `record_adjacent_track_gaps(...)`，只从高置信一一对应行收集严格递增的源列间距。
- Produces: `add_adjacent_gap_supported_candidates(...)`，把间距命中的两端登记为互相引用的原子候选。
- Renames: `AssignmentEvidence::span_partners` → `atomic_partners`，同时承载矩形跨度和相邻轨道间距的成对证据。
- Preserves: `AsciiDiagramCell`、`AsciiDiagramRow`、`AsciiDiagram`、`AsciiDiagramRegistry` 的现有公开/准公开字段与方法形状。

- [x] **Step 1: 加入完整 Electron/Go 布局失败回归**

在测试模块的 `OPEN_TIMELINE` 常量之后加入真实夹具：

```rust
const ELECTRON_GO_ARCHITECTURE: &[&str] = &[
    "Electron (UI Shell)          Go (Agent Core)",
    "┌──────────────────────┐    ┌─────────────────────────────┐",
    "│  Main Process         │    │  WebSocket Server            │",
    "│  ├─ spawn Go 二进制    │◄──►│  ├─ token 认证               │",
    "│  └─ BrowserWindow     │ WS │  └─ 收发 JSON 消息           │",
    "│                       │    │                              │",
    "│  Renderer (Chat UI)   │    │  Agent Loop                  │",
    "│  ├─ 流式对话           │    │  ├─ Orchestrator (主 agent)  │",
    "│  ├─ 工具调用卡片        │    │  └─ Worker (子 agent)       │",
    "│  ├─ 子 agent 进度      │    │                              │",
    "│  └─ 任务面板            │    │  LLM Provider               │",
    "└──────────────────────┘    │  ├─ Anthropic (流式 SSE)      │",
    "                            │  └─ OpenAI 兼容               │",
    "                            │                               │",
    "                            │  工具系统 (8 tools)            │",
    "                            │  条件式 Prompt 构建            │",
    "                            │  Skills 系统                  │",
    "                            │  会话持久化                    │",
    "                            └─────────────────────────────┘",
];
```

在 `cell_at_source_column()` 和 `right_edge_cell()` 辅助函数之后加入：

```rust
#[test]
fn aligns_electron_shell_outer_right_edge_across_tree_branch_rows() {
    const LEFT_FRAME_RIGHT_SOURCES: [(usize, usize); 11] = [
        (1, 23),
        (2, 24),
        (3, 25),
        (4, 24),
        (5, 24),
        (6, 24),
        (7, 25),
        (8, 26),
        (9, 25),
        (10, 26),
        (11, 23),
    ];

    let diagram = detect_ascii_diagram(&lines(ELECTRON_GO_ARCHITECTURE))
        .expect("the architecture fixture must be detected");
    let render_columns = LEFT_FRAME_RIGHT_SOURCES
        .map(|(row_index, source_column)| {
            cell_at_source_column(&diagram.rows[row_index], source_column).render_column()
        });

    assert_eq!(render_columns, [26; LEFT_FRAME_RIGHT_SOURCES.len()]);
    for row_index in [3, 7, 8, 9] {
        assert_eq!(
            cell_at_source_column(&diagram.rows[row_index], 3).render_column(),
            3,
            "tree branch on row {row_index} must stay at its source column"
        );
    }
}
```

- [x] **Step 2: 运行真实回归并确认 RED**

Run:

```bash
cargo test -p textora-markdown layout::ascii_diagram::tests::aligns_electron_shell_outer_right_edge_across_tree_branch_rows -- --exact
```

Expected: FAIL；左外框右边界实际包含第 25、26 两个 `render_column`，第 3、7、9 行断言期望 26、实际 25。

- [x] **Step 3: 加入相邻距离身份隔离和多解降级测试**

在低置信候选测试附近加入：

```rust
#[test]
fn adjacent_gap_candidates_are_scoped_to_their_track_pair() {
    let (row, _, _) = grid_row("├    │    │");
    let cell_indices = vertical_edge_cell_indices(&row);
    let expected_track_indices = [0, 1, 2];
    let supported_adjacent_gaps = BTreeMap::from([(
        AdjacentTrackPair { left_track_index: 1, right_track_index: 2 },
        BTreeSet::from([5]),
    )]);
    let mut candidates = BTreeMap::new();

    add_adjacent_gap_supported_candidates(
        &row,
        &expected_track_indices,
        &cell_indices,
        &supported_adjacent_gaps,
        &mut candidates,
    );

    let left = CandidatePosition { expected_position: 1, cell_position: 0 };
    let right = CandidatePosition { expected_position: 2, cell_position: 1 };
    assert!(candidates[&left].atomic_partners.contains(&right));
    assert!(candidates[&right].atomic_partners.contains(&left));
    assert!(candidates.keys().all(|position| position.expected_position != 0));
}

#[test]
fn ambiguous_adjacent_gap_pairs_remain_unassigned() {
    let (row, _, _) = grid_row("│    │    │");
    let cell_indices = vertical_edge_cell_indices(&row);
    let tracks = [BorderSide::Left, BorderSide::Right]
        .map(|side| VerticalBorderTrack { rectangle_index: 0, side, members: Vec::new() })
        .to_vec();
    let supported_adjacent_gaps = BTreeMap::from([(
        AdjacentTrackPair { left_track_index: 0, right_track_index: 1 },
        BTreeSet::from([5]),
    )]);

    let assignments = uniquely_supported_row_assignments(
        &row,
        &[0, 1],
        &cell_indices,
        &tracks,
        &[BTreeSet::new(), BTreeSet::new()],
        &[BTreeSet::new()],
        &supported_adjacent_gaps,
    );

    assert!(assignments.is_empty(), "two equal gap pairs must remain ambiguous");
}
```

- [x] **Step 4: 运行新增证据测试并确认编译 RED**

Run:

```bash
cargo test -p textora-markdown adjacent_gap -- --nocapture
```

Expected: 编译失败；`AdjacentTrackPair`、`add_adjacent_gap_supported_candidates` 和 `atomic_partners` 尚不存在，`uniquely_supported_row_assignments` 也尚未接收相邻间距参数。

- [x] **Step 5: 定义相邻轨道身份并泛化原子伙伴命名**

在 `CandidatePosition` 后加入：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AdjacentTrackPair {
    left_track_index: usize,
    right_track_index: usize,
}

type SupportedAdjacentGaps = BTreeMap<AdjacentTrackPair, BTreeSet<usize>>;
```

将证据字段改为：

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct AssignmentEvidence {
    directly_supported: bool,
    atomic_partners: BTreeSet<CandidatePosition>,
}
```

完成机械重命名：

```text
record_span_candidate_pair          → record_atomic_candidate_pair
span_partners                       → atomic_partners
has_complete_forced_span_partner    → has_complete_forced_atomic_partner
unique_reciprocal_span_partner      → unique_reciprocal_atomic_partner
```

重命名后函数行为保持不变；只泛化“伙伴来自矩形跨度”的旧命名，使其可同时表达相邻间距证据。

- [x] **Step 6: 从高置信行记录带身份的相邻间距**

在 `record_confident_row()` 前加入：

```rust
fn record_adjacent_track_gaps(
    row: &AsciiDiagramRow,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
    supported_adjacent_gaps: &mut SupportedAdjacentGaps,
) {
    for (expected_pair, cell_pair) in
        expected_track_indices.windows(2).zip(cell_indices.windows(2))
    {
        let left_column = row.cells[cell_pair[0]].column;
        let right_column = row.cells[cell_pair[1]].column;
        let Some(gap) = right_column.checked_sub(left_column) else {
            continue;
        };
        if gap == 0 {
            continue;
        }
        supported_adjacent_gaps
            .entry(AdjacentTrackPair {
                left_track_index: expected_pair[0],
                right_track_index: expected_pair[1],
            })
            .or_default()
            .insert(gap);
    }
}
```

给 `record_confident_row()` 增加参数：

```rust
supported_adjacent_gaps: &mut SupportedAdjacentGaps,
```

在该函数完成单轨列和矩形跨度记录后调用：

```rust
record_adjacent_track_gaps(
    row,
    expected_track_indices,
    cell_indices,
    supported_adjacent_gaps,
);
```

在 `vertical_border_tracks()` 初始化并传递集合：

```rust
let mut supported_adjacent_gaps = SupportedAdjacentGaps::new();
```

- [x] **Step 7: 为低置信行生成相邻间距原子候选**

在 `add_span_supported_candidates()` 后加入：

```rust
fn add_adjacent_gap_supported_candidates(
    row: &AsciiDiagramRow,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
    supported_adjacent_gaps: &SupportedAdjacentGaps,
    candidates: &mut BTreeMap<CandidatePosition, AssignmentEvidence>,
) {
    for (left_expected_position, expected_pair) in
        expected_track_indices.windows(2).enumerate()
    {
        let pair = AdjacentTrackPair {
            left_track_index: expected_pair[0],
            right_track_index: expected_pair[1],
        };
        let Some(supported_gaps) = supported_adjacent_gaps.get(&pair) else {
            continue;
        };

        for left_cell_position in 0..cell_indices.len() {
            let left_column = row.cells[cell_indices[left_cell_position]].column;
            for right_cell_position in left_cell_position + 1..cell_indices.len() {
                let right_column = row.cells[cell_indices[right_cell_position]].column;
                let Some(gap) = right_column.checked_sub(left_column) else {
                    continue;
                };
                if !supported_gaps.contains(&gap) {
                    continue;
                }
                record_atomic_candidate_pair(
                    candidates,
                    CandidatePosition {
                        expected_position: left_expected_position,
                        cell_position: left_cell_position,
                    },
                    CandidatePosition {
                        expected_position: left_expected_position + 1,
                        cell_position: right_cell_position,
                    },
                );
            }
        }
    }
}
```

给以下函数逐层增加 `supported_adjacent_gaps: &SupportedAdjacentGaps` 参数：

```text
supported_row_candidates
uniquely_supported_row_assignments
record_uniquely_supported_row
```

在 `supported_row_candidates()` 中，完成 direct 和矩形跨度候选构造后调用：

```rust
add_adjacent_gap_supported_candidates(
    row,
    expected_track_indices,
    cell_indices,
    supported_adjacent_gaps,
    &mut candidates,
);
```

在 `vertical_border_tracks()` 处理 `uncertain_rows` 时传入 `&supported_adjacent_gaps`。

- [x] **Step 8: 更新原子伙伴验证和复杂度说明**

将伙伴完整性过滤改为泛化后的函数：

```rust
fn has_complete_forced_atomic_partner(
    position: CandidatePosition,
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
    forced_positions: &BTreeSet<CandidatePosition>,
) -> bool {
    candidates[&position].atomic_partners.iter().any(|partner| {
        forced_positions.contains(partner)
            && candidates
                .get(partner)
                .is_some_and(|evidence| evidence.atomic_partners.contains(&position))
    })
}
```

`unique_reciprocal_atomic_partner()` 同样读取 `atomic_partners`。最终归属过滤保持：

```rust
candidates[position].directly_supported
    || has_complete_forced_atomic_partner(*position, &candidates, &forced_positions)
```

把 `uniquely_supported_row_assignments()` 上方复杂度说明改为：

```rust
/// Builds `K <= expected_tracks * cells` candidates once. Direct evidence costs
/// `O(expected_tracks * cells * log S)`. Rectangle-span evidence costs
/// `O(cells * sum(supported_spans_per_rectangle) * log cells)`. Adjacent-gap evidence costs
/// `O(expected_adjacent_pairs * cells^2 * log G)`. Ordered-map insertion adds logarithmic
/// factors. Uniqueness costs `O(K log K)`, complete-set crossing rejection costs `O(K^2)`,
/// and atomic-partner validation costs `O(P log K)` for `P` partner relationships. Storage is
/// `O(K + P)`; filtering never rebuilds or re-uniquifies the candidate set.
```

- [x] **Step 9: 修正现有单元测试调用点并运行格式化**

所有直接调用 `supported_row_candidates()` 或 `uniquely_supported_row_assignments()` 的既有测试，补传：

```rust
&SupportedAdjacentGaps::new()
```

只有新增相邻间距测试传入非空集合。然后运行：

```bash
cargo fmt --all
```

Expected: 命令成功；`cargo fmt --all -- --check` 随后无差异。

- [x] **Step 10: 运行布局定向测试并确认 GREEN**

Run:

```bash
cargo test -p textora-markdown layout::ascii_diagram::tests::aligns_electron_shell_outer_right_edge_across_tree_branch_rows -- --exact
cargo test -p textora-markdown adjacent_gap -- --nocapture
cargo test -p textora-markdown layout::ascii_diagram::tests
```

Expected:

- 真实 Electron/Go 回归 PASS，左框右边全部为第 26 列；
- 身份隔离和多解降级测试 PASS；
- 现有 ASCII 图测试全部 PASS，测试数量由当前 50 项增加到至少 53 项。

- [x] **Step 11: 编译并提交布局修复**

Run:

```bash
cargo check -p textora-markdown
git diff --check
git status --short
```

Expected: 编译成功、无 whitespace error；状态只包含本计划文档、设计文档和 `ascii_diagram.rs` 的预期修改。

Commit:

```bash
git add crates/markdown/src/layout/ascii_diagram.rs docs/plans/2026-07-16-ascii-diagram-tree-branch-border-alignment.md docs/plans/2026-07-16-ascii-diagram-tree-branch-border-alignment-implementation.md
git commit -m "fix(markdown): align frame borders across tree branches"
```

---

### Task 2: 增加最终像素回归并完成全面验证

**Files:**

- Modify: `crates/markdown/src/render.rs:1360-1815`
- Test: `crates/markdown/src/render.rs:1360-1815`

**Interfaces:**

- Consumes: Task 1 产出的 `render_column()` 对齐结果。
- Preserves: `render_ascii_diagram_row()`、`draw_box_connections()`、`DrawList` 命令形状和所有渲染 API。
- Produces: `ELECTRON_GO_ARCHITECTURE_DIAGRAM` 测试夹具及像素级回归测试。

- [x] **Step 1: 加入 fenced Markdown 渲染夹具**

在现有 `WPS_ARCHITECTURE_DIAGRAM` 常量之前加入：

```rust
const ELECTRON_GO_ARCHITECTURE_DIAGRAM: &str = r#"```
Electron (UI Shell)          Go (Agent Core)
┌──────────────────────┐    ┌─────────────────────────────┐
│  Main Process         │    │  WebSocket Server            │
│  ├─ spawn Go 二进制    │◄──►│  ├─ token 认证               │
│  └─ BrowserWindow     │ WS │  └─ 收发 JSON 消息           │
│                       │    │                              │
│  Renderer (Chat UI)   │    │  Agent Loop                  │
│  ├─ 流式对话           │    │  ├─ Orchestrator (主 agent)  │
│  ├─ 工具调用卡片        │    │  └─ Worker (子 agent)       │
│  ├─ 子 agent 进度      │    │                              │
│  └─ 任务面板            │    │  LLM Provider               │
└──────────────────────┘    │  ├─ Anthropic (流式 SSE)      │
                            │  └─ OpenAI 兼容               │
                            │                               │
                            │  工具系统 (8 tools)            │
                            │  条件式 Prompt 构建            │
                            │  Skills 系统                  │
                            │  会话持久化                    │
                            └─────────────────────────────┘
```"#;
```

- [x] **Step 2: 加入两个外框和连接箭头的像素回归**

在 `render_snapped_rectangle_uses_one_right_edge_x()` 后加入：

```rust
#[test]
fn render_electron_go_architecture_keeps_outer_tracks_and_arrow_separate() {
    const LEFT_FRAME_START_ROW: usize = 1;
    const LEFT_FRAME_END_ROW: usize = 11;
    const EXPECTED_LEFT_RIGHT_TRACK_INDEX: usize = 1;
    const EXPECTED_SERVER_LEFT_TRACK_INDEX: usize = 2;
    const EXPECTED_SERVER_RIGHT_TRACK_INDEX: usize = 3;

    let layout = build_laid_out(ELECTRON_GO_ARCHITECTURE_DIAGRAM);
    let code_block = layout.doc.blocks.first().expect("fixture has one code block");
    let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
        panic!("fixture must produce a code block");
    };
    assert_eq!(lines.len(), 19, "fixture must retain every architecture row");

    let draw_list = render_laid_out(&layout, 2_000.0);
    let top_border_xs = vertical_border_center_xs_for_line(&draw_list, &lines[1]);
    assert_eq!(top_border_xs.len(), 4, "top row must expose both outer rectangles");
    let left_right_x = top_border_xs[EXPECTED_LEFT_RIGHT_TRACK_INDEX];
    let server_left_x = top_border_xs[EXPECTED_SERVER_LEFT_TRACK_INDEX];
    let server_right_x = top_border_xs[EXPECTED_SERVER_RIGHT_TRACK_INDEX];

    for (frame_row, line) in lines[LEFT_FRAME_START_ROW..=LEFT_FRAME_END_ROW].iter().enumerate() {
        let border_xs = vertical_border_center_xs_for_line(&draw_list, line);
        assert_contains_x(
            &border_xs,
            left_right_x,
            &format!("left outer right track row {frame_row}"),
        );
    }

    for (row_index, line) in lines[1..].iter().enumerate() {
        let border_xs = vertical_border_center_xs_for_line(&draw_list, line);
        assert_contains_x(
            &border_xs,
            server_left_x,
            &format!("server left track row {row_index}"),
        );
        assert_contains_x(
            &border_xs,
            server_right_x,
            &format!("server right track row {row_index}"),
        );
    }

    let arrow_centers = line_text_center_xs(&draw_list, &lines[3], "►");
    assert_eq!(arrow_centers.len(), 1, "connector row must draw one right arrow head");
    assert!(
        left_right_x < arrow_centers[0] && arrow_centers[0] < server_left_x,
        "arrow must stay between the frames: left={left_right_x}, arrow={}, right={server_left_x}",
        arrow_centers[0]
    );
}
```

实现时若 `UiTextLayout` 将 `◄──►` 的非框线字符拆成不同命令，测试只定位 `►`；不得为了方便修改生产渲染分词。

- [x] **Step 3: 运行像素回归**

Run:

```bash
cargo test -p textora-markdown render::tests::render_electron_go_architecture_keeps_outer_tracks_and_arrow_separate -- --exact --nocapture
```

Expected: PASS。该测试验证 Task 1 的布局结果已自然传递到现有渲染器；不应为此修改任何生产渲染函数。

- [x] **Step 4: 运行 crate 级格式、编译和测试验证**

Run:

```bash
cargo fmt --all -- --check
cargo check -p textora-markdown
cargo test -p textora-markdown
git diff --check
```

Expected: 全部成功；没有新增 warning 或失败测试。

- [x] **Step 5: 运行项目全面验证**

Run:

```bash
./scripts/verify.sh
```

Expected: 脚本退出码为 0。若失败，记录首个失败命令和完整错误；只修复由本次改动引入的问题，不顺带处理无关历史失败。

- [x] **Step 6: 检查修改范围并提交像素回归**

Run:

```bash
git status --short
git diff --stat HEAD~1
git diff --check
```

Expected: 本阶段只新增 `render.rs` 测试；生产渲染逻辑无修改，工作区无意外文件。

Commit:

```bash
git add crates/markdown/src/render.rs
git commit -m "test(markdown): cover tree branch frame alignment"
```

## 最终完成条件

- 两个任务的定向测试、`textora-markdown` 全量测试和 `./scripts/verify.sh` 全部通过。
- Electron/Go 图表左外框右边界在布局列和最终像素 x 两个层面共线。
- 树形分支保持源列；右侧外框及连接箭头没有回归。
- 生产改动仅位于 `ascii_diagram.rs` 的证据收集与低置信候选构造路径。
- 两次提交边界清晰：第一提交为布局修复及方案文档，第二提交仅为最终像素回归。

## 实施补充：树形分支行的唯一相邻链

`add_adjacent_gap_supported_candidates()` 仅处理含 `├` 或 `┤` 的行。`add_span_supported_candidates()` 保持原有行为，只有在同一树形分支行中，跨度端点与直接支持冲突、两个预期位置相邻，并且跨度端点实际交叉一条唯一且直接兼容的相邻间距候选对时，才跳过该跨度候选。普通框图不生成相邻间距候选，也不触发这条跨度协作限制；`positions_without_crossing_candidates()`、双向唯一性和边界锚点逻辑均不变。
