# Mindmap Connector Width Inheritance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让思维导图拖拽预览线匹配目标位置的真实连线宽度。

**Architecture:** 保持普通连线的 5/3/2/1dp 层级规则不变。拖拽反馈通过 `target_rect` 在布局树中查找目标父节点，以 `parent.depth + 1` 复用现有连线宽度计算。

**Tech Stack:** Rust、`ui::core::paint::DrawList`、Cargo 单元测试

## Global Constraints

- 普通连线继续使用现有 5/3/2/1dp 层级规则。
- 拖拽预览线使用与放下后真实连线相同的起始宽度。
- 保留现有圆角路径、缩放行为、颜色和拖拽反馈。
- 保留 `crates/markdown/src/mmf/canvas.rs` 中已有未提交的缩放采样修复。

---

### Task 1: 修正拖拽预览连线起始宽度

**Files:**
- Modify: `crates/markdown/src/mmf/canvas.rs:215-228,1565-1598`
- Test: `crates/markdown/src/mmf/canvas.rs`

**Interfaces:**
- Consumes: `connector_head_width(depth: u8, reference_width: f32) -> f32` 与 `connector_tail_width(reference_width: f32) -> f32`
- Produces: 根据目标父节点深度计算拖拽预览线宽度

- [x] **Step 1: 在拖拽反馈测试中加入目标父节点宽度断言**

```rust
assert_eq!(
    first_guide_sample.w.min(first_guide_sample.h),
    SECOND_LEVEL_CONNECTOR_HEAD_WIDTH_DP,
    "drag guide should match a real connector from the non-root parent",
);
```

- [x] **Step 2: 运行测试并确认预览仍按一级宽度绘制而失败**

Run: `cargo test -p textora-markdown --lib drag_preview_draws_valid_insertion_feedback_and_invalid_color_without_insertion -- --nocapture`

Expected: FAIL，实际起始宽度为 5dp，而期望为 3dp。

- [x] **Step 3: 从目标父节点计算预览线宽度**

```rust
fn drag_guide_head_width(
    preview: &CanvasDragPreview,
    layout: &LayoutTree,
    reference_width: f32,
) -> f32 {
    let target_parent_depth = preview.target_rect.and_then(|target_rect| {
        layout.nodes.iter()
            .find(|layout_node| layout_rect(layout_node) == target_rect)
            .map(|layout_node| layout_node.depth)
    });
    let Some(target_parent_depth) = target_parent_depth else {
        return connector_tail_width(reference_width);
    };
    connector_head_width(target_parent_depth.saturating_add(1), reference_width)
}
```

`render_drag_target_feedback` 接收 `layout` 并使用该 helper；普通连线宽度计算保持不变。

- [x] **Step 4: 运行定向测试并确认通过**

Run: `cargo test -p textora-markdown --lib drag_preview_draws_valid_insertion_feedback_and_invalid_color_without_insertion -- --nocapture`

Expected: PASS。

- [ ] **Step 5: 格式化并运行画布相关回归测试**

Run: `cargo fmt --all -- --check`

Expected: PASS。

Run: `cargo test -p textora-markdown --lib mmf::canvas::tests -- --nocapture`

Expected: PASS，包含现有缩放采样回归测试。

- [ ] **Step 6: 编译验证**

Run: `cargo check -p textora-markdown`

Expected: PASS，无编译错误。
