# mmap 折叠控件悬浮、背景与居中实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让展开态的 `−` 控件仅在所属节点悬浮时出现，让收起态数字控件持续可见，并通过背景、圆环内边距和统一居中布局保证文字清晰可读。

**Architecture:** 改动限制在 `textora-markdown` 的 mmap 布局与画布渲染。布局只扩大控件命中尺寸以容纳当前字号；画布按节点分别判断悬浮可见性，并按“背景圆 → 内缩圆环 → 居中标签”的顺序绘制。现有源码事务、折叠计数、拖拽预览和 app 层协议不变。

**Tech Stack:** Rust、`textora-markdown`、`ui::canvas::DrawList`、`ui::core::geom::Rect`、现有 mmap 单元测试。

## Global Constraints

- 展开态 `−` 仅在所属节点卡片或自身控件区域悬浮时显示；不得因悬浮其他节点而显示。
- 收起态数字控件始终显示，数字仍表示完整后代总数。
- 控件背景必须使用 `theme.mindmap.canvas.background`，并在圆环之前绘制以遮挡底层连接线。
- `−` 与数字必须使用同一个居中函数，并按栅格字形像素的透明度加权视觉重心绝对居中。
- 字号保持当前 `1.0` 倍，不允许通过缩小文字解决圆环重叠。
- 根节点与叶节点仍不产生控件，拖拽预览期间仍隐藏控件。
- 不新增 app 层状态或 mmap 专用跨层结构。

---

### Task 1: 按节点状态与悬浮位置控制可见性

**Files:**
- Modify: `crates/markdown/src/mmf/canvas.rs:200-260,940-960,1239-1305`
- Test: `crates/markdown/src/mmf/canvas.rs`

**Interfaces:**
- Consumes: `MindmapRenderProjection::canvas_pointer`、`LayoutTree`、`ControlHitGeometry::bounds`、`NodeProps::collapsed`。
- Produces: `control_is_visible(node, layout, control, pointer) -> bool`；收起节点恒为 `true`，展开节点只在所属卡片或控件悬浮时为 `true`。

- [ ] **Step 1: 写入展开态无悬浮时隐藏的失败测试**

在 `canvas.rs` 测试模块新增：

```rust
#[test]
fn expanded_control_is_hidden_without_hover() {
    let constants = LayoutConstants::default();
    let theme = Theme::from_definition(&ThemeDefinition::default_dark());
    let layout = single_node_layout();
    let node = Node { title: "Branch".into(), ..empty_node(2) };
    let nodes = vec![&node];
    let control_bounds = Rect::new(90.0, 20.0, 24.0, 24.0);
    let hit_map = HitMap {
        nodes: Vec::new(),
        controls: vec![super::super::layout::ControlHitGeometry {
            source_node_index: 0,
            bounds: control_bounds,
        }],
        node_rects: Vec::new(),
        title_char_edges: Vec::new(),
    };
    let projection = plain_projection(["Branch"]);
    let mut draw_list = DrawList::new();
    let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

    render_cards_and_connectors(
        &mut draw_list,
        &layout,
        test_viewport(
            Rect::new(0.0, 0.0, 400.0, 200.0),
            Rect::new(0.0, 0.0, 400.0, 200.0),
        ),
        &theme,
        &constants,
        &mut shaper,
        &nodes,
        Some(&hit_map),
        &projection,
    );

    assert!(!draw_list.cmds.iter().any(|command| matches!(
        command,
        DrawCmd::TextLayout { layout, .. } if layout.text == "-"
    )));
}
```

- [ ] **Step 2: 写入单节点悬浮隔离的失败测试**

新增 `hovering_one_card_shows_only_its_expanded_control`，构造两个可见展开节点和两个控件，把 `canvas_pointer` 放在第一个节点卡片中心，最后断言：

```rust
let visible_minus_count = draw_list
    .cmds
    .iter()
    .filter(|command| matches!(
        command,
        DrawCmd::TextLayout { layout, .. } if layout.text == "-"
    ))
    .count();
assert_eq!(visible_minus_count, 1);
```

两个控件的 `source_node_index` 必须分别为 `0`、`1`；指针只落入 source `0` 的 `LayoutNode` 卡片，不得落入任一控件矩形。

- [ ] **Step 3: 验证两个测试按预期失败**

Run: `cargo test -p textora-markdown --lib mmf::canvas::tests::expanded_control_is_hidden_without_hover`

Expected: FAIL，旧实现无指针时仍绘制 `-`。

Run: `cargo test -p textora-markdown --lib mmf::canvas::tests::hovering_one_card_shows_only_its_expanded_control`

Expected: FAIL，旧实现会绘制两个 `-`。

- [ ] **Step 4: 实现按节点判断的可见性函数**

恢复 `render_controls` 的 `layout: &LayoutTree` 参数，将屏幕指针转换为内容坐标，并新增：

```rust
fn control_is_visible(
    node: &Node,
    layout: &LayoutTree,
    control: &super::layout::ControlHitGeometry,
    pointer: Option<CanvasPoint>,
) -> bool {
    if node.props.as_ref().is_some_and(|props| props.collapsed) {
        return true;
    }
    let Some(pointer) = pointer else {
        return false;
    };
    if control.bounds.contains(pointer.x, pointer.y) {
        return true;
    }
    layout_node_for_source(layout, control.source_node_index)
        .is_some_and(|layout_node| layout_rect(layout_node).contains(pointer.x, pointer.y))
}
```

在 `render_controls` 循环中取得 `node` 后立即提前返回当前迭代：

```rust
if !control_is_visible(node, layout, control, pointer) {
    continue;
}
```

调用方重新传入 `layout`。禁止恢复旧的“任意卡片悬浮即显示所有控件”全局布尔值。

- [ ] **Step 5: 验证可见性测试和既有行为**

Run: `cargo test -p textora-markdown --lib mmf::canvas::tests::expanded_control_is_hidden_without_hover && cargo test -p textora-markdown --lib mmf::canvas::tests::hovering_one_card_shows_only_its_expanded_control && cargo test -p textora-markdown --lib mmf::canvas::tests::controls_are_visible_without_pointer_and_show_collapsed_descendant_count && cargo test -p textora-markdown --lib mindmap_view::tests::card_hover_highlights_fill_border_and_control_symbol && cargo test -p textora-markdown --lib mindmap_view::tests::control_hover_does_not_highlight_its_card`

Expected: 5 个测试全部 PASS。

- [ ] **Step 6: 编译检查并提交**

Run: `cargo fmt --check && cargo check -p textora-markdown`

Expected: 格式与编译检查均通过且无警告。

Run: `git add crates/markdown/src/mmf/canvas.rs && git commit -m "fix(mindmap): scope collapse controls to node hover"`

Expected: 提交只包含控件可见性与对应测试。

### Task 2: 增加背景、圆环内边距与统一居中布局

**Files:**
- Modify: `crates/markdown/src/mmf/layout.rs:8-12,444-477,775-805`
- Modify: `crates/markdown/src/mmf/canvas.rs:29-38,200-260,1239-1305`
- Test: `crates/markdown/src/mmf/layout.rs`
- Test: `crates/markdown/src/mmf/canvas.rs`

**Interfaces:**
- Consumes: `ControlHitGeometry::bounds`、`theme.mindmap.canvas.background`、`measure_text()`、`viewport.zoom`。
- Produces: `centered_control_label_position(control_rect, label, font_size, shaper) -> (f32, f32)`，以及固定 `36dp` 控件空间、`2dp` 圆环内缩和不低于当前 `1.0` 倍的标签字号。

- [ ] **Step 1: 写入控件尺寸的失败测试**

在 `layout.rs` 的 `controls_exclude_root_and_leaves_and_use_shared_child_turn_point` 末尾增加：

```rust
assert_eq!(control.bounds.w, 36.0);
assert_eq!(control.bounds.h, 36.0);
```

- [ ] **Step 2: 写入背景、圆环内缩和标签居中的失败测试**

将现有 `controls_are_visible_without_pointer_and_show_collapsed_descendant_count` 扩展为检查以下结果。控件会做不足半像素的视觉校准，因此先从背景绘制命令取得最终可见范围，再计算内缩圆环：

```rust
let control_rect = rendered_control_bounds(&draw_list, logical_control_rect, background);
let inset = CONTROL_RING_INSET_DP * viewport.zoom;
let expected_ring_rect = control_rect.shrink(inset, inset, inset, inset);
```

然后断言背景圆和内缩圆环存在：

```rust
assert!(draw_list.cmds.iter().any(|command| matches!(
    command,
    DrawCmd::FillRect { rect, color, radius }
        if *rect == control_rect
            && *color == theme.mindmap.canvas.background
            && (*radius - control_rect.w.min(control_rect.h) * 0.5).abs() < f32::EPSILON
)));
assert!(draw_list.cmds.iter().any(|command| matches!(
    command,
    DrawCmd::StrokeRect { rect, color, .. }
        if *rect == expected_ring_rect
            && *color == theme.mindmap.canvas.connector
)));
```

找到数字 `3` 的文本命令，按真实 `split_subpixel` 相位与最终像素取整重新栅格化，并断言 alpha 加权视觉重心与可见圆环中心的误差不超过 `0.05px`。同一断言必须覆盖 `-`，以及 `1.0`、`1.75` 两种缩放：

```rust
let label_center = draw_list
    .cmds
    .iter()
    .find_map(|command| match command {
        DrawCmd::TextLayout { layout, x, y_baseline, .. } if layout.text == "3" => {
            Some(label_visual_center(layout, *x, *y_baseline, &mut shaper))
        }
        _ => None,
    })
    .expect("collapsed count label");
assert_label_visual_center_is_centered(label_center, control_rect);
```

同一测试再把节点改为展开态、把 `canvas_pointer` 设置为该节点卡片中心后重跑居中断言，确保可见的 `-` 与数字共用同一坐标规则。

- [ ] **Step 3: 验证视觉几何测试按预期失败**

Run: `cargo test -p textora-markdown --lib mmf::layout::tests::controls_exclude_root_and_leaves_and_use_shared_child_turn_point`

Expected: FAIL，旧控件尺寸为 `24dp`。

Run: `cargo test -p textora-markdown --lib mmf::canvas::tests::controls_are_visible_without_pointer_and_show_collapsed_descendant_count`

Expected: FAIL，旧实现没有背景圆且圆环没有内缩。

- [ ] **Step 4: 扩大控件空间并实现具名视觉常量**

在 `layout.rs` 中将：

```rust
const CONTROL_HIT_SIZE_DP: f32 = 24.0;
```

改为：

```rust
const CONTROL_HIT_SIZE_DP: f32 = 36.0;
```

在 `canvas.rs` 的常量区新增并保留当前可读字号：

```rust
const CONTROL_RING_INSET_DP: f32 = 2.0;
const CONTROL_FONT_SIZE_MULTIPLIER: f32 = 1.0;
const CONTROL_LABEL_SUBPIXEL_PHASE_COUNT: u8 = 4;
const CONTROL_LABEL_SUBPIXEL_STEP: f32 = 0.25;
```

- [ ] **Step 5: 实现背景、内缩圆环和统一居中函数**

`centered_control_label_position` 先 shape 一次标签，再枚举渲染器的 4 个横向子像素 phase。每个候选都按 `paint_backend` 的规则栅格化并取整 glyph 位置，计算 alpha 加权视觉重心，最终选择横向误差最小的候选；纵向选择最接近逻辑圆心的像素基线。由于像素取整后的字形重心不一定能落在任意小数坐标上，可见圆环与背景再对齐最终字形重心，命中范围和连接点保持不变。

在 `render_controls` 内按以下顺序绘制：

```rust
let label_placement = centered_control_label_position(control_rect, &label, font_size, shaper);
let control_center = CanvasPoint::new(
    control_rect.x + control_rect.w * 0.5,
    control_rect.y + control_rect.h * 0.5,
);
let visual_control_rect = Rect::new(
    control_rect.x + label_placement.visual_center.x - control_center.x,
    control_rect.y + label_placement.visual_center.y - control_center.y,
    control_rect.w,
    control_rect.h,
);
let control_radius =
    visual_control_rect.w.min(visual_control_rect.h) * CONTROL_CIRCLE_RADIUS_RATIO;
dl.fill_rounded(visual_control_rect, theme.mindmap.canvas.background, control_radius);

let ring_inset = CONTROL_RING_INSET_DP * viewport.zoom;
let ring_rect = visual_control_rect.shrink(ring_inset, ring_inset, ring_inset, ring_inset);
let ring_radius = ring_rect.w.min(ring_rect.h) * CONTROL_CIRCLE_RADIUS_RATIO;
dl.stroke_rounded(
    ring_rect,
    theme.mindmap.canvas.connector,
    ring_radius,
    CONTROL_RING_BORDER_WIDTH * viewport.zoom,
);

dl.text_shaped(
    label_placement.text_x,
    label_placement.baseline,
    font_size,
    theme.mindmap.canvas.connector,
    &label,
    shaper,
);
```

不得根据数字位数降低 `font_size`；本任务通过扩大控件空间和内缩圆环保留可读性。

- [ ] **Step 6: 运行目标测试与全量验证**

Run: `cargo test -p textora-markdown --lib mmf::layout::tests::controls_exclude_root_and_leaves_and_use_shared_child_turn_point && cargo test -p textora-markdown --lib mmf::canvas::tests::controls_are_visible_without_pointer_and_show_collapsed_descendant_count`

Expected: 两个测试 PASS。

Run: `cargo fmt --check && cargo test -p textora-markdown --lib && cargo check -p textora-markdown`

Expected: `textora-markdown` 全量测试、格式和编译检查全部通过，无警告。

Run: `./scripts/verify.sh`

Expected: workspace 格式检查、clippy warnings-as-errors 和 workspace 全量测试全部通过。

- [ ] **Step 7: 提交视觉几何修改**

Run: `git add crates/markdown/src/mmf/layout.rs crates/markdown/src/mmf/canvas.rs && git commit -m "fix(mindmap): center readable collapse control labels"`

Expected: 提交只包含控件尺寸、背景、圆环内缩、居中函数和对应测试。

## 后续校正（2026-07-15）

根据实际截图复核，补充以下视觉校正：

- 控件命中/绘制空间最终扩大为 `36dp`；保持 `2dp` 内缩后，可见圆环直径为 `32dp`。
- 圆环与标签使用 `connector`，与连接线保持同色。
- 标签定位依据实际栅格字形像素的透明度加权视觉重心，而非字距、固定基线或外接矩形；`-` 与数字都以控件中心为准。
- 新增缩放居中、连接线颜色，以及数字和 `-` 的视觉重心居中测试，并完成工作区全量校验。
