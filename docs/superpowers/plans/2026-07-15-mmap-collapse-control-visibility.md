# mmap 折叠控件可见性实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 mmap 的折叠控件持续可见、以圆环和更大文字呈现，并以收起节点的完整后代数替代加号，同时强化卡片悬浮描边。

**Architecture:** 仅调整 `textora-markdown` 的布局宽度与画布渲染。布局不再为收起节点标题预留后代数字；画布始终绘制既有命中图提供的控件，并从投影中的完整 DFS 后代计数决定控件标签。悬浮态继续复用主题的 `connector_hover` 色，仅增加具名描边宽度。

**Tech Stack:** Rust、`textora-markdown`、`ui::canvas::DrawList`、现有 mmap 单元测试。

## Global Constraints

- UI 层不得依赖或访问 app 层状态；本次仅使用 `MindmapRenderProjection`、`HitMap` 和 `Node` 纯数据。
- 根节点和叶节点不得生成折叠控件，现有控件命中优先级不变。
- 收起态数字必须是完整后代总数，不是直接子节点数。
- 所有新增尺寸均使用具名常量，禁止在渲染逻辑中写无语义数值。
- 每项按 Red → Green → Refactor 执行，提交前运行 `cargo fmt` 与编译检查。

---

### Task 1: 移除收起标题中的重复后代计数

**Files:**
- Modify: `crates/markdown/src/mmf/layout.rs:10-11,229-249,750-773`
- Test: `crates/markdown/src/mmf/layout.rs:750-773`

**Interfaces:**
- Consumes: `collect_card_widths_by_depth(node, depth, source_node_index, constants, shaper, projected_title, out)`。
- Produces: 收起与展开节点在标题相同且无投影替换时使用相同的卡片宽度；后代计数只留给渲染投影供控件使用。

- [ ] **Step 1: 写入失败测试**

将 `collapsed_card_width_includes_descendant_count_suffix` 重命名为 `collapsed_card_width_matches_expanded_title_width`，保留同一棵收起/展开树夹具，将断言替换为：

```rust
assert!(
    (collapsed_child.w - expanded_child.w).abs() < f32::EPSILON,
    "collapsed title must not reserve descendant-count width"
);
```

- [ ] **Step 2: 验证测试按预期失败**

Run: `cargo test -p textora-markdown --lib mmf::layout::tests::collapsed_card_width_matches_expanded_title_width`

Expected: FAIL，原因是旧实现将 `" · N"` 拼接进收起节点标题后再计算宽度。

- [ ] **Step 3: 实现最小修改**

删除 `COLLAPSED_LABEL_SEPARATOR` 常量，并将 `collect_card_widths_by_depth` 中的分支替换为：

```rust
let card_w = measured_card_width(title, constants, shaper);
```

保留随后对 `is_expanded` 的遍历截断，确保折叠依旧隐藏后代而不改变布局树语义。

- [ ] **Step 4: 验证通过**

Run: `cargo test -p textora-markdown --lib mmf::layout::tests::collapsed_card_width_matches_expanded_title_width`

Expected: PASS。

- [ ] **Step 5: 格式化并提交该独立修改**

Run: `cargo fmt --check && git add crates/markdown/src/mmf/layout.rs && git commit -m "fix(mindmap): keep collapsed counts out of titles"`

Expected: 格式检查通过，并产生只包含布局与测试的提交。

### Task 2: 绘制持续可见的圆环控件并强化卡片悬浮描边

**Files:**
- Modify: `crates/markdown/src/mmf/canvas.rs:31-32,158-238,515-545,1133-1245,1325-1345`
- Test: `crates/markdown/src/mmf/canvas.rs:1133-1245,1325-1345`

**Interfaces:**
- Consumes: `HitMap::controls` 的控件几何、`MindmapRenderProjection::collapsed_descendant_counts` 的完整后代数和 `NodeProps::collapsed` 状态。
- Produces: `render_controls` 始终为 `HitMap::controls` 画圆环及状态标签；`render_cards_with_hover` 在卡片命中时使用加粗描边；`render_text` 不再渲染 `" · N"` 后缀。

- [ ] **Step 1: 写入三个失败测试**

在 `canvas.rs` 的测试模块中：

1. 将 `non_hover_canvas_does_not_draw_collapse_control_symbols` 替换为 `controls_are_visible_without_pointer_and_show_collapsed_descendant_count`。夹具使用含子节点的 `Node`、`collapsed: true`、`collapsed_descendant_counts: vec![Some(3)]` 和 `canvas_pointer: None`，断言存在 `layout.text == "3"`，且存在一个 `StrokeRect`，其 `radius` 等于控件屏幕矩形半宽。
2. 将 `collapsed_suffix_uses_screen_scaled_title_width_at_zoom_levels` 替换为 `collapsed_title_omits_descendant_suffix_at_zoom_levels`。每个缩放级别断言标题 `"Branch"` 被绘制，且不存在 `layout.text == " · 3"`。
3. 在已有卡片悬浮渲染夹具旁新增 `hovered_card_border_is_wider_than_default_border`：分别以 `None` 和卡片中心指针调用 `render_cards_with_hover`，筛选对应卡片的 `DrawCmd::StrokeRect`，并断言悬浮描边宽度大于默认描边宽度。

- [ ] **Step 2: 验证三个测试按预期失败**

Run: `cargo test -p textora-markdown --lib "mmf::canvas::tests::controls_are_visible_without_pointer_and_show_collapsed_descendant_count"`

Expected: FAIL，旧控件在无指针时直接返回，且显示 `+`。

Run: `cargo test -p textora-markdown --lib "mmf::canvas::tests::collapsed_title_omits_descendant_suffix_at_zoom_levels"`

Expected: FAIL，旧文本渲染会输出 `" · 3"`。

Run: `cargo test -p textora-markdown --lib "mmf::canvas::tests::hovered_card_border_is_wider_than_default_border"`

Expected: FAIL，旧悬浮与默认卡片均传入同样的 `viewport.zoom` 描边宽度。

- [ ] **Step 3: 实现最小渲染修改**

在文件常量区定义语义化尺寸：

```rust
const DEFAULT_CARD_BORDER_WIDTH: f32 = 1.0;
const HOVERED_CARD_BORDER_WIDTH: f32 = 2.0;
const CONTROL_RING_BORDER_WIDTH: f32 = 2.0;
const CONTROL_FONT_SIZE_MULTIPLIER: f32 = 1.0;
```

在 `render_cards_with_hover` 中先选择 `border_width`，再传入描边：

```rust
let border_width = if hovered {
    HOVERED_CARD_BORDER_WIDTH
} else {
    DEFAULT_CARD_BORDER_WIDTH
};

dl.stroke_rounded(
    rect,
    with_alpha(border, opacity),
    constants.card_radius * viewport.zoom,
    border_width * viewport.zoom,
);
```

将 `render_controls` 改为不接收或检查 `pointer`，遍历所有 `hit_map.controls`。每个控件先绘制圆环，再绘制标签：

```rust
let label = if node.props.as_ref().is_some_and(|props| props.collapsed) {
    projection
        .collapsed_descendant_counts
        .get(control.source_node_index)
        .copied()
        .flatten()
        .map(|count| count.to_string())
        .unwrap_or_default()
} else {
    "-".to_owned()
};
dl.stroke_rounded(
    control_rect,
    theme.mindmap.canvas.connector_hover,
    control_rect.w.min(control_rect.h) * 0.5,
    CONTROL_RING_BORDER_WIDTH * viewport.zoom,
);
```

将 `projection` 作为 `render_controls` 参数传入，并删除 `render_text` 中生成、测量及绘制 `" · {descendant_count}"` 的整个分支。拖拽预览的 `"标题 · N"` 标签不在本任务范围内，必须保留。

- [ ] **Step 4: 验证新测试及相关回归测试通过**

Run: `cargo test -p textora-markdown --lib "mmf::canvas::tests::controls_are_visible_without_pointer_and_show_collapsed_descendant_count" && cargo test -p textora-markdown --lib "mmf::canvas::tests::collapsed_title_omits_descendant_suffix_at_zoom_levels" && cargo test -p textora-markdown --lib "mmf::canvas::tests::hovered_card_border_is_wider_than_default_border" && cargo test -p textora-markdown --lib mindmap_view::tests::card_hover_highlights_fill_border_and_control_symbol && cargo test -p textora-markdown --lib mindmap_view::tests::control_hover_does_not_highlight_its_card`

Expected: 全部 PASS；最后两项证明控件独立悬浮不高亮卡片的既有语义仍然存在。

- [ ] **Step 5: 完整验证、格式化并提交**

Run: `cargo fmt --check && cargo test -p textora-markdown --lib && cargo check -p textora-markdown`

Expected: 三个命令均以退出码 0 完成。

Run: `git add crates/markdown/src/mmf/canvas.rs && git commit -m "fix(mindmap): clarify collapse controls and hover"`

Expected: 提交只包含控件和悬浮渲染及其测试。

### Task 3: 更新 app 层控件端到端断言

**Files:**
- Modify: `crates/app/src/dispatch/mouse.rs:1051-1062`

**Interfaces:**
- Consumes: app 层已有 mmap 控件点击与画布渲染端到端夹具。
- Produces: 跨层测试验证收起后标题保持纯标题文本，计数 `2` 由持续可见的控件绘制。

- [ ] **Step 1: 更新回归断言**

在 `canvas_control_end_to_end` 中保留 `Parent` 断言，将旧断言：

```rust
assert!(rendered_text.contains(&" · 2"));
```

替换为：

```rust
assert!(rendered_text.contains(&"2"));
assert!(!rendered_text.contains(&" · 2"));
```

- [ ] **Step 2: 验证 app 层测试**

Run: `cargo test -p textora-app --lib dispatch::mouse::tests::canvas_control_end_to_end`

Expected: PASS。

- [ ] **Step 3: 最终格式化与集成验证**

Run: `./scripts/verify.sh`

Expected: 格式检查、`cargo clippy --workspace --all-targets -- -D warnings` 和 workspace 全量测试均通过。
