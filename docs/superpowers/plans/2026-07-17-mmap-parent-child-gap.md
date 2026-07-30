# mmap 父子节点横向间距 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 mmap 根到一级节点保持 35dp 横向边缘间距，其余父子节点保持 25dp，且不再受同层标题宽度影响。

**Architecture:** 将主题与布局常量中的列距 `level_indent` 替换为两个父子边缘间距。布局递归以父卡片的实际右边缘为子节点 x 坐标的基准，连接线和控件命中继续从这些实际坐标推导。

**Tech Stack:** Rust、textora-ui 主题解析、textora-markdown MMF 布局、cargo test。

## Global Constraints

- UI 层只提供纯主题数据；mmap 的树与布局状态继续留在 `textora-markdown`。
- 根到一级节点间距固定为 35dp；其余父子节点间距固定为 25dp。
- 同级纵向间距、节点尺寸、缩放与连接线视觉样式不变。
- 禁止使用 `.unwrap()`；测试中的不可恢复前提使用说明明确的 `.expect(...)`。
- 每个生产改动必须先有失败测试，且在修改实现前确认该测试因缺少目标行为而失败。

---

## 文件结构

| 文件 | 职责 |
| --- | --- |
| `crates/ui/src/theme/mindmap.rs` | 定义语义化的 mmap 父子间距默认值。 |
| `crates/ui/src/theme_file.rs` | 从 TOML 主题文件解析并校验两项间距覆盖值。 |
| `crates/markdown/src/mmf/layout.rs` | 用父节点右边缘和间距计算子节点坐标、连接线转折点及折叠控件位置。 |
| `crates/markdown/src/mindmap_view.rs` | 将主题几何值按 DPI 映射为布局常量。 |

### Task 1: 主题几何参数与 TOML 覆盖

**Files:**

- Modify: `crates/ui/src/theme/mindmap.rs:61-75,123-139,340-365`
- Modify: `crates/ui/src/theme_file.rs:183-198,497-515,695-723`

**Interfaces:**

- Produces: `MindmapGeometry::{root_child_gap, nested_child_gap}: f32`，默认分别为 `35.0` 和 `25.0`。
- Produces: `MindmapGeometryFile::{root_child_gap, nested_child_gap}: Option<f32>`；仅接受有限且不小于 `0.0` 的 TOML 值。
- Consumes: `ThemeFile::resolve(&ThemeDefinition) -> Result<ThemeDefinition, ResolveError>`。

- [ ] **Step 1: 写入主题默认值与 TOML 覆盖的失败测试**

在 `theme/mindmap.rs` 测试模块增加：

```rust
#[test]
fn default_geometry_uses_compact_parent_child_gaps() {
    let geometry = MindmapGeometry::default();

    assert_eq!(geometry.root_child_gap, 35.0);
    assert_eq!(geometry.nested_child_gap, 25.0);
}
```

在 `theme_file.rs` 的 `resolve_mindmap_drag_feedback_overrides` TOML 的 `[mindmap.geometry]` 段增加：

```toml
root_child_gap = 42.0
nested_child_gap = 18.0
```

并在既有断言后增加：

```rust
assert_eq!(resolved.mindmap.geometry.root_child_gap, 42.0);
assert_eq!(resolved.mindmap.geometry.nested_child_gap, 18.0);
```

- [ ] **Step 2: 运行测试并确认 RED**

Run: `cargo test -p textora-ui --lib default_geometry_uses_compact_parent_child_gaps -- --exact`

Expected: FAIL，编译器提示 `MindmapGeometry` 尚无 `root_child_gap` 与 `nested_child_gap` 字段。

- [ ] **Step 3: 实现最小的主题与配置接口**

将 `MindmapGeometry` 的 `level_indent` 替换为：

```rust
pub root_child_gap: f32,
pub nested_child_gap: f32,
```

在 `Default` 中设置：

```rust
root_child_gap: 35.0,
nested_child_gap: 25.0,
```

将 `MindmapGeometryFile::level_indent` 替换为同名的两个 `Option<f32>` 字段，并在 `resolve_mindmap()` 使用：

```rust
apply_geometry_f32(&mut target.geometry.root_child_gap, g.root_child_gap, 0.0);
apply_geometry_f32(
    &mut target.geometry.nested_child_gap,
    g.nested_child_gap,
    0.0,
);
```

不要保留无消费者的 `level_indent` 字段，也不要把旧值转换为新值；主题文件的未知键策略继续由 `deny_unknown_fields` 明确拒绝。

- [ ] **Step 4: 运行 UI 测试并确认 GREEN**

Run: `cargo test -p textora-ui --lib default_geometry_uses_compact_parent_child_gaps -- --exact && cargo test -p textora-ui --lib resolve_mindmap_drag_feedback_overrides -- --exact`

Expected: 两个测试均通过。

- [ ] **Step 5: 格式化并提交主题接口**

Run: `cargo fmt --check && cargo test -p textora-ui --lib mindmap`

Expected: 格式检查和 mmap 相关 UI 测试均通过。

```bash
git add crates/ui/src/theme/mindmap.rs crates/ui/src/theme_file.rs
git commit -m "feat(ui): configure mmap parent child gaps"
```

### Task 2: 按父节点边缘布局与 DPI 映射

**Files:**

- Modify: `crates/markdown/src/mmf/layout.rs:8-54,160-221,266-336,610-625,790-845`
- Modify: `crates/markdown/src/mindmap_view.rs:146-165`

**Interfaces:**

- Consumes: `LayoutConstants::{root_child_gap, nested_child_gap}`，均已按 DPI 缩放。
- Produces: `LayoutConstants::child_gap_for_parent_depth(parent_depth: u8) -> f32`，根节点返回 `root_child_gap`，其他节点返回 `nested_child_gap`。
- Produces: `assign_positions()` 以传入的节点 x 坐标布局当前节点；子节点 x 为 `parent.x + parent.w + child_gap_for_parent_depth(parent.depth)`。

- [ ] **Step 1: 写入父子边缘间距的失败布局测试**

在 `layout.rs` 测试模块增加：

```rust
#[test]
fn parent_child_edge_gaps_are_fixed_despite_wide_titles() {
    let tree = parser::parse(
        "# Root\n## An intentionally wide first-level title\n### Child\n#### Grandchild\n",
    )
    .expect("fixture must be valid MMF");
    let constants = LayoutConstants {
        root_child_gap: 35.0,
        nested_child_gap: 25.0,
        ..LayoutConstants::default()
    };
    let mut shaper = Shaper::new().expect("test shaper should initialize");
    let layout = compute_layout(&tree, &mut shaper, &constants, None);

    let root = &layout.nodes[0];
    let first_level = &layout.nodes[1];
    let second_level = &layout.nodes[2];
    let third_level = &layout.nodes[3];

    assert!(((first_level.x - (root.x + root.w)) - 35.0).abs() < 0.01);
    assert!(((second_level.x - (first_level.x + first_level.w)) - 25.0).abs() < 0.01);
    assert!(((third_level.x - (second_level.x + second_level.w)) - 25.0).abs() < 0.01);
}
```

同时更新 `scaled_constants_make_mindmap_less_dense`，断言 `LayoutConstants::scaled(2.0)` 的两项间距分别为 `70.0` 和 `50.0`；更新折叠控件测试，令预期转折点使用 `constants.nested_child_gap / 2.0`。

- [ ] **Step 2: 运行测试并确认 RED**

Run: `cargo test -p textora-markdown --lib parent_child_edge_gaps_are_fixed_despite_wide_titles -- --exact`

Expected: FAIL，测试使用的 `LayoutConstants` 字段尚未定义，或现有按深度列距的布局不能满足 35dp/25dp 断言。

- [ ] **Step 3: 实现按父节点右边缘的最小布局改造**

在 `LayoutConstants` 中将 `level_indent` 替换为两个间距字段，并实现：

```rust
pub fn child_gap_for_parent_depth(&self, parent_depth: u8) -> f32 {
    if parent_depth == 0 {
        self.root_child_gap
    } else {
        self.nested_child_gap
    }
}
```

删除 `MIN_CONNECTOR_HORIZONTAL_GAP_DP` 与 `depth_x_positions()`。将 `assign_positions()` 的 `depth_x_positions: &[f32]` 参数替换为当前节点的 `x: f32`；在递归子节点前计算：

```rust
let child_x = x + card_w + constants.child_gap_for_parent_depth(depth);
let child_connector_turn_x = (this_connector.0 + child_x) * 0.5;
```

将 `compute_layout()` 中的根调用设为 `x = 0.0`。保留每层统一卡片宽度的测量逻辑，但不再用它决定横坐标。

折叠节点没有可见子节点时，控件回退转折点改为：

```rust
layout_node.x
    + layout_node.w
    + constants.child_gap_for_parent_depth(layout_node.depth) * 0.5
```

在 `MindmapView::update_layout_constants()` 将主题字段映射为：

```rust
root_child_gap: geometry.root_child_gap * dpi_scale,
nested_child_gap: geometry.nested_child_gap * dpi_scale,
```

- [ ] **Step 4: 运行 markdown 回归测试并确认 GREEN**

Run: `cargo test -p textora-markdown --lib parent_child_edge_gaps_are_fixed_despite_wide_titles -- --exact && cargo test -p textora-markdown --lib wide_parent_keeps_grandchild_connector_pointing_right -- --exact && cargo test -p textora-markdown --lib collapsed_node_expand_control_offset_remains_eight_dp_at_high_dpi -- --exact`

Expected: 三个测试全部通过；第一个测试确认宽标题不改变 35dp/25dp 间距，后两个测试确认连接线与折叠控件仍使用有效的正向连接线几何。

- [ ] **Step 5: 格式化、完整验证并提交布局改造**

Run: `cargo fmt --check && cargo test -p textora-markdown --lib && cargo check -p textora-markdown && cargo check -p textora-app`

Expected: 所有命令退出码为 0。

```bash
git add crates/markdown/src/mmf/layout.rs crates/markdown/src/mindmap_view.rs
git commit -m "feat(markdown): use fixed mmap parent child gaps"
```

### Task 3: 最终集成验证

**Files:**

- Verify only: `crates/ui/src/theme/mindmap.rs`
- Verify only: `crates/ui/src/theme_file.rs`
- Verify only: `crates/markdown/src/mmf/layout.rs`
- Verify only: `crates/markdown/src/mindmap_view.rs`

**Interfaces:**

- Consumes: Task 1 的主题字段与 Task 2 的布局映射。
- Produces: 可复现的 workspace 级验证结果。

- [ ] **Step 1: 检查工作树只包含预期改动**

Run: `git status --short && git diff --check`

Expected: 无空白错误；任何与本任务无关的已有改动不暂存、不修改，也不纳入提交。

- [ ] **Step 2: 运行项目全量验证脚本**

Run: `./scripts/verify.sh`

Expected: 格式、Clippy（warnings as errors）和 workspace 测试均通过。

- [ ] **Step 3: 手动验证视觉结果**

打开包含根、一级、二级和三级节点的 `.mmap.md`，其中一级标题使用明显较长文本。确认根→一级卡片边缘间距为 35dp，一级→二级与二级→三级均为 25dp；修改标题长度后，这三类边缘间距保持不变。

- [ ] **Step 4: 记录验证结果**

在交付说明中列出执行的测试命令、实际退出状态，以及任何未由本任务引入的既有工作树改动；不新增功能性提交。

