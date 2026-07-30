# mmap 折叠控件悬浮反馈 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 mmap 节点卡片与其展开/收起控件共享 hover 状态，指针位于任一命中区时同步显示主题强调色。

**Architecture:** 指针位置已经通过 `MindmapRenderProjection::canvas_pointer` 进入画布渲染。画布在内容坐标中用同一个“卡片矩形或控件矩形”命中函数计算节点状态，卡片渲染与两种控件绘制共享该结果；高亮颜色继续使用 `theme.mindmap.canvas.connector_hover`。

**Tech Stack:** Rust、`textora-markdown`、`ui::canvas::DrawList`、既有 mmap 渲染单元测试。

## Global Constraints

- 控件命中使用现有 `ControlHitGeometry::bounds`，不改变布局与点击范围。
- 仅控件圆环和内部减号/后代数量使用 `connector_hover`；控件背景保持 `canvas.background`。
- 指针位于节点卡片或其控件的任一命中区时，卡片与控件必须同步高亮。
- 展开态与收起态必须使用同一悬停判定，根节点、叶节点和拖拽预览的既有可见性规则不变。
- 不新增主题项、跨层状态或魔法值。

---

### Task 1: 为控件悬停颜色建立回归测试

**Files:**
- Modify: `crates/markdown/src/mmf/canvas.rs:208-315,1800-1915`
- Test: `crates/markdown/src/mmf/canvas.rs`

**Interfaces:**
- Consumes: `MindmapRenderProjection::canvas_pointer`、`ControlHitGeometry::bounds`、`Theme::mindmap.canvas.connector_hover`。
- Produces: 控件悬停时包含强调色圆环与符号的 `DrawList` 命令。

- [ ] **Step 1: 写入收起态控件的失败测试**

在 `expanded_control_is_hidden_without_hover` 后添加测试，使用现有 `single_node_layout`、`plain_projection`、`test_viewport` 与 `render_cards_and_connectors` 辅助函数：

```rust
#[test]
fn collapsed_control_hover_uses_hover_color_for_ring_and_label() {
    const CONTROL_HOVER_COLOR: [f32; 4] = [0.2, 0.6, 0.3, 1.0];
    let constants = LayoutConstants::default();
    let mut theme = Theme::from_definition(&ThemeDefinition::default_dark());
    theme.mindmap.canvas.connector_hover = CONTROL_HOVER_COLOR;
    let layout = single_node_layout();
    let mut node = Node { title: "Branch".into(), ..empty_node(2) };
    node.props = Some(super::super::model::NodeProps {
        id: None,
        priority: None,
        status: None,
        owner: None,
        collapsed: true,
        tags: Vec::new(),
        color: None,
    });
    let nodes = vec![&node];
    let control_bounds = Rect::new(90.0, 20.0, 36.0, 36.0);
    let hit_map = HitMap {
        nodes: Vec::new(),
        controls: vec![super::super::layout::ControlHitGeometry {
            source_node_index: 0,
            bounds: control_bounds,
        }],
        node_rects: Vec::new(),
        title_char_edges: Vec::new(),
    };
    let mut projection = plain_projection(["Branch"]);
    projection.collapsed_descendant_counts = vec![Some(4)];
    projection.canvas_pointer = Some(CanvasPoint::new(108.0, 38.0));
    let mut draw_list = DrawList::new();
    let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

    render_cards_and_connectors(
        &mut draw_list,
        &layout,
        test_viewport(Rect::new(0.0, 0.0, 400.0, 200.0), Rect::new(0.0, 0.0, 400.0, 200.0)),
        &theme,
        &constants,
        &mut shaper,
        &nodes,
        Some(&hit_map),
        &projection,
    );

    assert!(draw_list.cmds.iter().any(|command| matches!(
        command,
        DrawCmd::StrokeRect { rect, color, .. }
            if *rect == control_bounds.shrink(2.0, 2.0, 2.0, 2.0)
                && *color == CONTROL_HOVER_COLOR
    )));
    assert!(draw_list.cmds.iter().any(|command| matches!(
        command,
        DrawCmd::TextLayout { layout, color, .. }
            if layout.text == "4" && *color == CONTROL_HOVER_COLOR
    )));
}
```

- [ ] **Step 2: 运行测试，确认失败原因是缺失悬停颜色**

Run: `cargo test -p textora-markdown --lib mmf::canvas::tests::collapsed_control_hover_uses_hover_color_for_ring_and_label`

Expected: FAIL；圆环和标签仍为 `theme.mindmap.canvas.connector`。

### Task 2: 将控件命中状态传入绘制并使用主题悬停色

**Files:**
- Modify: `crates/markdown/src/mmf/canvas.rs:208-315`
- Test: `crates/markdown/src/mmf/canvas.rs`

**Interfaces:**
- Consumes: `pointer: Option<CanvasPoint>`（内容坐标）、`control.bounds`、`Theme`。
- Produces: `control_is_hovered(control, pointer) -> bool`；`render_collapsed_control`、`render_expanded_control`、`render_control_chrome` 均接收 `hovered: bool`。

- [ ] **Step 1: 添加仅判断控件边界的纯函数**

在 `render_controls` 前添加：

```rust
fn control_is_hovered(
    control: &super::layout::ControlHitGeometry,
    pointer: Option<CanvasPoint>,
) -> bool {
    pointer.is_some_and(|point| control.bounds.contains(point.x, point.y))
}
```

- [ ] **Step 2: 在控件循环中计算悬停状态并透传**

在 `render_controls` 的 `control_rect` 后添加：

```rust
let hovered = control_is_hovered(control, pointer);
```

将两个调用分别改为：

```rust
render_collapsed_control(
    dl, control_rect, &label, font_size, theme, shaper, viewport, hovered,
);
render_expanded_control(dl, control_rect, theme, viewport, hovered);
```

并为 `render_collapsed_control`、`render_expanded_control` 与 `render_control_chrome` 增加末尾参数 `hovered: bool`，逐层传递。

- [ ] **Step 3: 统一控件圆环和符号颜色**

在 `render_control_chrome` 的起始处添加：

```rust
let control_color = if hovered {
    theme.mindmap.canvas.connector_hover
} else {
    theme.mindmap.canvas.connector
};
```

用 `control_color` 取代圆环 `stroke_rounded` 的 `theme.mindmap.canvas.connector`。在收起态 `dl.text_shaped` 与展开态 `dl.fill` 中使用同样的条件表达式作为颜色；不得改变背景圆的 `theme.mindmap.canvas.background`。

- [ ] **Step 4: 运行新增测试，确认转绿**

Run: `cargo test -p textora-markdown --lib mmf::canvas::tests::collapsed_control_hover_uses_hover_color_for_ring_and_label`

Expected: PASS。

- [ ] **Step 5: 运行控件可见性与卡片隔离回归测试**

Run: `cargo test -p textora-markdown --lib mmf::canvas::tests::expanded_control_is_hidden_without_hover && cargo test -p textora-markdown --lib mmf::canvas::tests::hovering_one_card_shows_only_its_expanded_control && cargo test -p textora-markdown --lib mindmap_view::tests::control_hover_does_not_highlight_its_card`

Expected: 3 个测试均 PASS；展开控件可见性与“控件悬停不高亮卡片”行为不变。

- [ ] **Step 6: 格式化与编译验证**

Run: `cargo fmt --check && cargo check -p textora-markdown`

Expected: 两个命令退出码均为 0。

- [ ] **Step 7: 提交最小修复**

Run: `git add crates/markdown/src/mmf/canvas.rs docs/superpowers/plans/2026-07-15-mmap-collapse-control-hover.md && git commit -m "fix(mindmap): highlight collapse controls on hover"`

Expected: 提交只包含本计划与控件悬停实现、测试。

### Task 3: 让节点卡片与控件共享悬停命中

**Files:**
- Modify: `crates/markdown/src/mmf/canvas.rs:145-205,465-485,1160-1170`
- Modify: `crates/markdown/src/mindmap_view.rs:3272-3370`
- Test: `crates/markdown/src/mindmap_view.rs`

**Interfaces:**
- Consumes: `LayoutTree` 节点矩形、`ControlHitGeometry::bounds`、内容坐标指针。
- Produces: `pointer_hits_node_or_control(node_rect, control_bounds, pointer) -> bool`，并由卡片与控件渲染共同使用。

- [ ] **Step 1: 先把两条现有测试改成新行为并确认红灯**

将卡片 hover 测试中的控件颜色期望改为：

```rust
assert!(draw_list_has_expanded_control_bar(
    &draw_list,
    control_screen_rect,
    theme.mindmap.canvas.connector_hover,
));
```

将 `control_hover_does_not_highlight_its_card` 重命名为 `control_hover_highlights_its_card_and_control`，并将卡片断言改为：

```rust
assert!(branch_fill_alpha.expect("hovered branch fill") > 0.5);
assert!(draw_list.cmds.iter().any(|command| matches!(
    command,
    DrawCmd::StrokeRect { rect, color, .. }
        if *rect == branch_screen_rect && *color == theme.mindmap.canvas.connector_hover
)));
assert!(draw_list_has_expanded_control_bar(
    &draw_list,
    control_screen_rect,
    theme.mindmap.canvas.connector_hover,
));
```

Run: `cargo test -p textora-markdown --lib mindmap_view::tests::card_hover_highlights_fill_border_and_control_symbol && cargo test -p textora-markdown --lib mindmap_view::tests::control_hover_highlights_its_card_and_control`

Expected: 两条测试均 FAIL，分别暴露卡片 hover 未传给控件、控件 hover 未传给卡片。

- [ ] **Step 2: 增加共享矩形命中函数**

在 `render_cards_with_hover` 前加入：

```rust
fn pointer_hits_node_or_control(
    node_rect: Rect,
    control_bounds: Option<Rect>,
    pointer: Option<CanvasPoint>,
) -> bool {
    pointer.is_some_and(|point| {
        node_rect.contains(point.x, point.y)
            || control_bounds.is_some_and(|bounds| bounds.contains(point.x, point.y))
    })
}
```

- [ ] **Step 3: 让卡片渲染接收同一份控件命中图**

为 `render_cards_with_hover` 增加 `hit_map: Option<&HitMap>` 参数；在循环中取当前节点控件矩形并改用：

```rust
let control_bounds = hit_map.and_then(|map| {
    map.controls
        .iter()
        .find(|control| control.source_node_index == ln.source_node_index)
        .map(|control| control.bounds)
});
let hovered = pointer_hits_node_or_control(layout_rect(ln), control_bounds, pointer);
```

`render_cards` 公共包装函数传入 `None`，`render_cards_and_connectors` 传入当前 `hit_map`。

- [ ] **Step 4: 让控件使用同一份命中判定**

将 `control_is_hovered` 改为接收所属节点矩形，并实现为：

```rust
fn control_is_hovered(
    layout: &LayoutTree,
    control: &super::layout::ControlHitGeometry,
    pointer: Option<CanvasPoint>,
) -> bool {
    let node_rect = layout_node_for_source(layout, control.source_node_index)
        .map(layout_rect)
        .unwrap_or(Rect::ZERO);
    pointer_hits_node_or_control(node_rect, Some(control.bounds), pointer)
}
```

在 `render_controls` 中调用 `control_is_hovered(layout, control, pointer)`。`control_is_visible` 也复用 `pointer_hits_node_or_control`，保持展开态只在所属节点或控件悬浮时显示。

- [ ] **Step 5: 运行两条回归测试确认转绿**

Run: `cargo test -p textora-markdown --lib mindmap_view::tests::card_hover_highlights_fill_border_and_control_symbol && cargo test -p textora-markdown --lib mindmap_view::tests::control_hover_highlights_its_card_and_control`

Expected: 两条测试 PASS。

- [ ] **Step 6: 运行完整验证**

Run: `cargo fmt --check`

Run: `cargo test -p textora-markdown --lib`

Run: `cargo check -p textora-markdown`

Expected: 格式检查退出码 0，709 条 markdown 单测通过，编译检查退出码 0。

## 自检

- 规格覆盖：Task 1 覆盖收起态后代计数控件；Task 2 将同一判定透传给展开态减号，并保持卡片隔离及既有可见性规则。
- 占位符：无未完成标记或未定义的实现步骤。
- 类型一致性：新函数只接收现有的 `ControlHitGeometry`、`CanvasPoint` 和 `bool`；绘制函数的新增参数均为 `hovered: bool`。
