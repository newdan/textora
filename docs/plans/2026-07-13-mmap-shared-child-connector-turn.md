# 同父节点连线共享拐点轴 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** 让每个父节点的直接子连接线在同一条竖向轴上转弯。

**Architecture:** 布局层为每条非根节点入边保存父节点为该组子节点计算的共享拐点 x 坐标。画布渲染层仅使用这个坐标构造圆角折线路径；无固定父子关系的拖拽引导线仍自行取中点。

**Tech Stack:** Rust、textora-markdown、现有 DrawList 单元测试。

## Global Constraints

- 产品名为 textora，Markdown crate 包名为 textora-markdown。
- 仅修改 crates/markdown/src/mmf/layout.rs 与 crates/markdown/src/mmf/canvas.rs 的生产代码和同文件测试。
- 根节点没有入边，connector_turn_x 必须为 None；非根节点必须由其父节点提供拐点轴。
- 不修改拖拽引导线的按端点中点路由规则。
- 每次提交前运行 cargo fmt --check 和 cargo check -p textora-markdown。

---

### Task 1: 在布局结果中保存同父节点的共享拐点轴

**Files:**

- Modify: crates/markdown/src/mmf/layout.rs:44-53
- Modify: crates/markdown/src/mmf/layout.rs:119-173
- Test: crates/markdown/src/mmf/layout.rs:tests

**Interfaces:**

- Consumes: depth_x_positions: &[f32]、父节点右侧端点 this_connector: (f32, f32)。
- Produces: LayoutNode::connector_turn_x: Option<f32>，供画布连接线渲染使用。

- [ ] **Step 1: 写出失败的布局回归测试**

在 layout.rs 的 tests 模块中新增：

~~~rust
#[test]
fn sibling_connectors_share_their_parent_turn_axis() {
    let tree = parser::parse("# Root\n## First\n## Second\n## Third\n")
        .expect("fixture must be valid MMF");
    let mut shaper = Shaper::new().expect("test shaper should initialize");
    let layout = compute_layout(&tree, &mut shaper, &LayoutConstants::default(), None);
    let root = &layout.nodes[0];
    let children = &layout.nodes[1..4];
    let turn_x = children[0]
        .connector_turn_x
        .expect("child connector must have a turn axis");

    assert!(root.connector_turn_x.is_none(), "root must not have an incoming connector axis");
    assert!(turn_x > children[0].connector_from.0);
    assert!(turn_x < children[0].connector_to.0);
    assert!(children.iter().all(|child| child.connector_turn_x == Some(turn_x)));
}
~~~

- [ ] **Step 2: 运行测试，确认它因缺少路由数据而失败**

Run: cargo test -p textora-markdown sibling_connectors_share_their_parent_turn_axis

Expected: 编译失败，提示 LayoutNode 没有 connector_turn_x 字段。

- [ ] **Step 3: 以最小改动实现共享轴**

在 LayoutNode 增加字段：

~~~rust
pub connector_turn_x: Option<f32>,
~~~

将 assign_positions 的参数扩展为：

~~~rust
parent_connector_from: Option<(f32, f32)>,
parent_connector_turn_x: Option<f32>,
~~~

构造当前 LayoutNode 时保存 parent_connector_turn_x。根调用传入两个 None。在递归每个子节点前，使用下一层左边缘与当前节点右边缘计算一次共享轴，并对全部子节点传递同一值：

~~~rust
let child_x = depth_x_positions[(depth + 1) as usize];
let child_connector_turn_x = (this_connector.0 + child_x) * 0.5;

assign_positions(
    child,
    depth + 1,
    cursor,
    node_idx,
    Some(this_connector),
    Some(child_connector_turn_x),
    constants,
    card_widths_by_depth,
    depth_x_positions,
    out,
);
~~~

为测试中的手工 LayoutNode 初始化补充 connector_turn_x，根节点使用 None。

- [ ] **Step 4: 运行布局回归测试，确认通过**

Run: cargo test -p textora-markdown sibling_connectors_share_their_parent_turn_axis

Expected: 1 passed; 0 failed。

- [ ] **Step 5: 格式化并验证本任务**

Run: cargo fmt --check && cargo check -p textora-markdown

Expected: 两个命令均以 exit code 0 结束。

- [ ] **Step 6: 提交布局路由数据**

~~~bash
git add crates/markdown/src/mmf/layout.rs
git commit -m "fix(mmap): share sibling connector turn axis"
~~~

### Task 2: 按布局提供的拐点轴生成连接线路径

**Files:**

- Modify: crates/markdown/src/mmf/canvas.rs:206-280
- Modify: crates/markdown/src/mmf/canvas.rs:render_drag_target_feedback
- Test: crates/markdown/src/mmf/canvas.rs:tests

**Interfaces:**

- Consumes: LayoutNode::connector_turn_x，其中非根节点为 Some(f32)。
- Produces: connector_centerline(from, to, turn_x, width)，其竖向段位于显式 turn_x。

- [ ] **Step 1: 写出失败的路径回归测试**

在 canvas.rs 的 tests 模块中新增：

~~~rust
#[test]
fn connector_centerline_uses_the_supplied_turn_axis() {
    let turn_x = 40.0;
    let points = connector_centerline((0.0, 0.0), (120.0, 120.0), turn_x, 8.0);

    assert!(points.windows(2).any(|segment| {
        (segment[0].0 - turn_x).abs() < ZERO_DISTANCE_EPSILON
            && (segment[1].0 - turn_x).abs() < ZERO_DISTANCE_EPSILON
            && (segment[0].1 - segment[1].1).abs() > ZERO_DISTANCE_EPSILON
    }));
}
~~~

- [ ] **Step 2: 运行测试，确认旧路径 API 不能表达显式拐点轴**

Run: cargo test -p textora-markdown connector_centerline_uses_the_supplied_turn_axis

Expected: 编译失败，提示 connector_centerline 参数数量不匹配。

- [ ] **Step 3: 以最小改动消费布局路由数据**

将路径函数签名改为：

~~~rust
fn connector_centerline(
    from: (f32, f32),
    to: (f32, f32),
    turn_x: f32,
    width: f32,
) -> Vec<(f32, f32)>
~~~

删除函数内的 mid_x 计算，并将现有 mid_x 用途改为 turn_x。在 draw_connector 读取：

~~~rust
let turn_x = ln
    .connector_turn_x
    .expect("non-root mindmap connector must receive a layout turn axis");
~~~

将 turn_x 传给 connector_centerline。拖拽引导线保留中点语义，在调用处显式计算：

~~~rust
let turn_x = (parent_anchor.x + child_preview.x) * 0.5;
let points = connector_centerline(
    (parent_anchor.x, parent_anchor.y),
    (child_preview.x, child_preview.y),
    turn_x,
    constants.connector_width * viewport.zoom,
);
~~~

更新现有测试和测试辅助 LayoutNode 初始化的 connector_turn_x：非根节点使用与旧中点相同的值，根节点使用 None。

- [ ] **Step 4: 运行路径回归测试，确认通过**

Run: cargo test -p textora-markdown connector_centerline_uses_the_supplied_turn_axis

Expected: 1 passed; 0 failed。

- [ ] **Step 5: 运行完整 crate 验证**

Run: cargo fmt --check && cargo test -p textora-markdown && cargo check -p textora-markdown

Expected: 格式检查、所有 Markdown crate 单元测试和编译检查均以 exit code 0 结束。

- [ ] **Step 6: 提交渲染修复**

~~~bash
git add crates/markdown/src/mmf/canvas.rs crates/markdown/src/mmf/layout.rs
git commit -m "fix(mmap): route child connectors through shared turn axis"
~~~
