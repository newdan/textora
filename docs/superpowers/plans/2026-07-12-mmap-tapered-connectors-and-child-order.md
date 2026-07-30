# MMF 渐变连线与跨父节点排序 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复长标题导致的反向渐变父子连线，并让拖入其他节点时按释放高度写入正确的子节点顺序。

**Architecture:** `mmf::layout` 在已测量各层最大卡片宽度后生成动态深度横坐标，保持现有 `connector_from → connector_to` 的圆角渐变绘制。`MindmapView` 在“成为目标子节点”分支中计算目标直接子节点的插入锚点；`mmf::edit` 以该锚点生成一个局部、原子的源码移动事务。

**Tech Stack:** Rust、`textora-markdown`、MMF 解析树、现有 `EditTransaction`、内联单元测试。

## Global Constraints

- 保留当前父端较粗、子端较细的平滑渐变连线；不得改为等宽共享树干或折线。
- 根节点不可移动，拖动子树、属性和备注必须原样保留。
- 同级拖放仍按锚点上方/下方决定前置或后置。
- 跨父节点时，释放高度决定目标直接子节点列表中的插入点；目标无子节点或释放点低于最后一个子节点时追加。
- 每次移动只产生一个 `EditTransaction` 和一个 Undo 单元。
- 不触碰用户已修改的 `test_data/sample.mmap.md`。
- 禁止 `.unwrap()`；必须运行 `cargo fmt`。

---

## 文件与职责

| 文件 | 职责 |
| --- | --- |
| `crates/markdown/src/mmf/layout.rs` | 根据每层真实卡片宽度生成不会反向的深度横坐标与直接父子连线端点。 |
| `crates/markdown/src/mindmap_view.rs` | 将跨父节点拖放的屏幕 Y 坐标转换为“指定子节点之前”或“最后一个子节点”的预览和编辑目标。 |
| `crates/markdown/src/mmf/edit.rs` | 用指定子节点范围生成跨父节点移动的单一源码事务。 |

## Task 1: 保持渐变风格并消除宽卡片反向连线

**Files:**

- Modify: `crates/markdown/src/mmf/layout.rs:8-232, 368-506`
- Test: `crates/markdown/src/mmf/layout.rs` 内 `#[cfg(test)]` 模块

**Interfaces:**

- Produces: `depth_x_positions(card_widths_by_depth, level_indent) -> Vec<f32>`，其中每个后继深度的左边界不小于上一层卡片右边缘加最小连线水平空间。
- Preserves: `LayoutNode::connector_from` 为直接父卡片右侧中点，`LayoutNode::connector_to` 为本卡片左侧中点；`canvas::draw_connector` 不修改。

- [ ] **Step 1: 写入宽父卡片的失败测试**

在 `layout.rs` 的测试模块中添加：

```rust
#[test]
fn wide_parent_keeps_grandchild_connector_pointing_right() {
    let tree = parser::parse(
        "# Root\n## A parent title that is deliberately wider than one level indent\n### Child\n",
    )
    .expect("fixture must be valid MMF");
    let mut shaper = Shaper::new().expect("test shaper should initialize");
    let layout = compute_layout(&tree, &mut shaper, &LayoutConstants::default(), None);
    let parent = &layout.nodes[1];
    let child = &layout.nodes[2];

    assert!(
        child.x > parent.x + parent.w,
        "child card must be right of its parent: parent_right={}, child_left={}",
        parent.x + parent.w,
        child.x,
    );
    assert!(
        child.connector_from.0 < child.connector_to.0,
        "tapered parent-to-child connector must point right",
    );
}
```

- [ ] **Step 2: 运行测试，确认当前固定层级步长会失败**

Run: `cargo test -p textora-markdown --lib wide_parent_keeps_grandchild_connector_pointing_right`

Expected: FAIL，断言显示 `child.x <= parent.x + parent.w`，证明连线端点可能反向。

- [ ] **Step 3: 以深度最大宽度计算横坐标**

在 `layout.rs` 的常量区加入具名的最小留白，并添加纯函数：

```rust
const MIN_CONNECTOR_HORIZONTAL_GAP_DP: f32 = 32.0;

fn depth_x_positions(card_widths_by_depth: &[f32], level_indent: f32) -> Vec<f32> {
    let mut positions = Vec::with_capacity(card_widths_by_depth.len());
    let mut x = 0.0;
    for width in card_widths_by_depth {
        positions.push(x);
        x += level_indent.max(*width + MIN_CONNECTOR_HORIZONTAL_GAP_DP);
    }
    positions
}
```

将 `assign_positions` 的 `x` 来源由 `depth as f32 * constants.level_indent` 改为参数 `depth_x_positions[depth as usize]`，并在 `compute_layout` 于 `collect_card_widths_by_depth` 后构造该数组、传入根递归调用。函数签名更新为：

```rust
fn assign_positions(
    node: &Node,
    depth: u8,
    y_offset: f32,
    node_idx: &mut usize,
    parent_connector_from: Option<(f32, f32)>,
    constants: &LayoutConstants,
    card_widths_by_depth: &[f32],
    depth_x_positions: &[f32],
    out: &mut Vec<LayoutNode>,
)
```

递归调用也原样传递 `depth_x_positions`。不要修改 `connector_from`、`connector_to`、`canvas::connector_centerline` 或渐变宽度常量。

- [ ] **Step 4: 验证失败测试转绿且既有渐变渲染测试仍通过**

Run: `cargo test -p textora-markdown --lib wide_parent_keeps_grandchild_connector_pointing_right`

Expected: PASS。

Run: `cargo test -p textora-markdown --lib connector_tapers_from_parent_to_child`

Expected: PASS，断言仍验证父端 `5dp`、子端 `1dp` 的渐变。

- [ ] **Step 5: 格式化并提交这一独立修复**

Run: `cargo fmt --check && cargo check -p textora-markdown`

Expected: 两个命令均以状态码 0 结束。

```bash
git add crates/markdown/src/mmf/layout.rs
git commit -m "fix(markdown): keep mmap connectors right of wide parents"
```

## Task 2: 按释放高度插入目标节点的子序列

**Files:**

- Modify: `crates/markdown/src/mindmap_view.rs:38-67, 351-440, 707-770, 1253-1467`
- Modify: `crates/markdown/src/mmf/edit.rs:9-13, 148-252, 600-930`
- Test: 两个文件内现有 `#[cfg(test)]` 模块

**Interfaces:**

- Produces: `MoveSubtreeTarget::BeforeChild`，其 `anchor_range` 是目标父节点的直接子节点范围。
- Produces: `child_drop_target(tree, layout, parent_index, drag_y) -> (Range<usize>, MoveSubtreeTarget)`；它选择首个垂直中心不低于指针的直接子节点，否则返回父节点范围与 `LastChild`。
- Consumes: `MindmapDragPreview { anchor_range, target, canvas }`；`CanvasDragPreview::target_rect` 继续表示目标父卡片，`insertion_line` 表示后继子节点上方。

- [ ] **Step 1: 为源码事务写入跨父节点的失败测试**

在 `edit.rs` 测试模块中添加：

```rust
#[test]
fn move_subtree_before_target_child_preserves_requested_child_order() {
    let source = "# Root\n## Source\n## Parent\n### First\n### Last\n";
    let tree = parser::parse(source).expect("fixture must be valid MMF");
    let plan = plan_move_subtree(
        &tree,
        source,
        node_range(&tree, "Source"),
        node_range(&tree, "Last"),
        MoveSubtreeTarget::BeforeChild,
        1,
    );

    assert_transaction_text_and_selection(
        plan,
        source,
        "# Root\n## Parent\n### First\n### Source\n### Last\n",
        "### Source\n",
    );
}
```

- [ ] **Step 2: 运行事务测试，确认枚举值尚不存在**

Run: `cargo test -p textora-markdown --lib move_subtree_before_target_child_preserves_requested_child_order`

Expected: FAIL，编译错误指出 `MoveSubtreeTarget::BeforeChild` 未定义。

- [ ] **Step 3: 以指定子节点为锚点生成原子事务**

扩展枚举和 `plan_move_subtree` 的两个 `match`：

```rust
pub(crate) enum MoveSubtreeTarget {
    BeforeSibling,
    AfterSibling,
    BeforeChild,
    LastChild,
}
```

`BeforeChild` 的目标标题级别等于 `anchor_node.heading_level`，插入位置等于 `anchor_node.subtree_source_range.start`，且不执行 `BeforeSibling`/`AfterSibling` 的同父节点拒绝逻辑。保留 `BeforeSibling` 与 `AfterSibling` 的现有同父校验，避免改变同级移动语义：

```rust
let sibling_offset = match target {
    MoveSubtreeTarget::BeforeSibling | MoveSubtreeTarget::AfterSibling => {
        let Some(source_parent) = find_parent(tree, source_index) else {
            return EditPlan::Consume;
        };
        let Some(anchor_parent) = find_parent(tree, anchor_index) else {
            return EditPlan::Consume;
        };
        if !std::ptr::eq(source_parent, anchor_parent) {
            return EditPlan::Consume;
        }
        0
    }
    MoveSubtreeTarget::BeforeChild => 0,
    MoveSubtreeTarget::LastChild => 1,
};

let insertion_byte = match target {
    MoveSubtreeTarget::BeforeSibling | MoveSubtreeTarget::BeforeChild => {
        anchor_node.subtree_source_range.start
    }
    MoveSubtreeTarget::AfterSibling | MoveSubtreeTarget::LastChild => {
        anchor_node.subtree_source_range.end
    }
};
```

同样在所有 `MoveSubtreeTarget` 穷尽匹配中为 `BeforeChild` 添加分支；只有 `BeforeSibling` 与 `AfterSibling` 调用同级无操作检测。

- [ ] **Step 4: 验证跨父节点事务和既有同级拒绝测试**

Run: `cargo test -p textora-markdown --lib move_subtree_before_target_child_preserves_requested_child_order`

Expected: PASS。

Run: `cargo test -p textora-markdown --lib move_subtree_rejects_sibling_target_with_a_different_parent`

Expected: PASS，证明 `BeforeSibling` 的跨父拒绝仍有效。

- [ ] **Step 5: 为拖放预览写入失败测试**

在 `mindmap_view.rs` 测试模块中添加：

```rust
#[test]
fn drag_into_parent_inserts_before_the_next_direct_child() {
    let source = "# Root\n## Source\n## Parent\n### First\n### Last\n";
    let (mut view, doc) = view_with_source(source);
    render_test_view(&mut view, &doc);
    let source_range = node_by_title(view.ready_tree(), "Source").subtree_source_range.clone();
    let parent = &view.ready_hit_map().nodes[2].card_rect;
    let first = &view.ready_hit_map().nodes[3].card_rect;
    let last = &view.ready_hit_map().nodes[4].card_rect;
    let request = drag_request(
        CanvasDragPhase::Update,
        source_range,
        parent.x + parent.w + 8.0,
        (first.y + first.h * 0.5 + last.y + last.h * 0.5) * 0.5,
        1,
    );

    let response = view.handle_canvas_drag(request, &doc);

    assert!(matches!(response, CanvasDragResponse::Preview(preview)
        if preview.is_valid
            && preview.target_rect == Some(*parent)
            && preview.insertion_line == Some(((last.x, last.y), (last.x + last.w, last.y)))));
    assert!(matches!(view.drag_state, MindmapDragState::Preview(MindmapDragPreview {
        target: Some(mmf::edit::MoveSubtreeTarget::BeforeChild),
        ..
    })));
}
```

- [ ] **Step 6: 运行预览测试，确认当前实现只会追加为最后一个子节点**

Run: `cargo test -p textora-markdown --lib drag_into_parent_inserts_before_the_next_direct_child`

Expected: FAIL，当前 `MindmapDragPreview.target` 为 `LastChild`，且没有目标子节点上方的插入线。

- [ ] **Step 7: 在 MindmapView 中计算直接子节点插入锚点**

增加一个纯辅助函数，使用 DFS 索引找出 `parent_index` 的直接子节点，并按布局 Y 中心决定后继锚点：

```rust
fn next_child_at_or_below_pointer(
    tree: &Tree,
    layout: &LayoutTree,
    parent_index: usize,
    pointer_y: f32,
) -> Option<usize> {
    let nodes = collect_nodes_dfs(&tree.root);
    (0..nodes.len())
        .filter(|node_index| {
            find_parent(tree, *node_index)
                .is_some_and(|parent| std::ptr::eq(parent, nodes[parent_index]))
        })
        .find(|node_index| {
            let child = &layout.nodes[*node_index];
            pointer_y < child.y + child.h * 0.5
        })
}
```

在 `calculate_drag_preview` 的非同级分支中：如果该函数返回子节点索引，保存该子节点的 `subtree_source_range`，目标设为 `BeforeChild`，并将 `insertion_line` 设为该子节点顶部；否则保留候选父节点范围和 `LastChild`。无论哪种子节点落点，`canvas.target_rect` 都保持候选父节点卡片，拖放引导线继续指向该父节点。

扩展 `insertion_line`：`BeforeChild` 与 `BeforeSibling` 都返回 `Some(((anchor.x, anchor.y), (anchor.x + anchor.w, anchor.y)))`。在 `is_noop_sibling_target` 中让 `BeforeChild` 返回 `false`。

- [ ] **Step 8: 验证预览、应用事务和回归集**

Run: `cargo test -p textora-markdown --lib drag_into_parent_inserts_before_the_next_direct_child`

Expected: PASS。

Run: `cargo test -p textora-markdown --lib drag_drop_applies_the_previewed_same_level_move`

Expected: PASS。

Run: `cargo test -p textora-markdown --lib`

Expected: 全部通过。

- [ ] **Step 9: 格式化、编译并提交这一独立修复**

Run: `cargo fmt --check && cargo check -p textora-markdown`

Expected: 两个命令均以状态码 0 结束。

```bash
git add crates/markdown/src/mindmap_view.rs crates/markdown/src/mmf/edit.rs
git commit -m "fix(markdown): order mmap children by drag position"
```

## 最终验证

- [ ] `cargo fmt --check`
- [ ] `cargo test -p textora-markdown --lib`
- [ ] `cargo check -p textora-markdown`
- [ ] 检查 `git status --short`，确认只保留用户先前的 `test_data/sample.mmap.md` 修改。
