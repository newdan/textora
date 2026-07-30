# Mindmap Drag Feedback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 使思维导图的同级节点可在原有卡片水平带内重排，并让拖拽光标和视觉反馈一致、清晰。

**Architecture:** 在 `MindmapView` 中先解析同级兄弟排序候选，再保留既有的跨层左侧候选路径。插入线计算扩展为读取相邻兄弟间隙；纯渲染层只使用该几何结果绘制细线与选中态焦点色。应用层只根据选择及拖拽会话状态选择手型光标。

**Tech Stack:** Rust、`textora-markdown`、`ui`、`winit`、内联单元测试。

## Global Constraints

- 不让 `ui` 访问 `app` 状态；不新增跨层依赖。
- 不改变首次点击只选中节点、再次拖动才开始移动的手势。
- 不新增主题文件字段；插入线高度使用语义化渲染常量。
- 所有生产改动先由失败测试覆盖，提交前运行 `cargo fmt` 与相关测试。

---

### Task 1: 同级拖拽候选与居中插入几何

**Files:**

- Modify: `crates/markdown/src/mindmap_view.rs:351-430,703-802,1329-1610`

**Interfaces:**

- Consumes: `LayoutTree`、`Tree`、`MoveSubtreeTarget`、`find_siblings`。
- Produces: `calculate_drag_preview()` 返回带有居中 `CanvasDragPreview::insertion_line` 的合法同级排序预览。

- [ ] **Step 1: 写入失败测试，覆盖一级同级节点在卡片水平带内排序**

```rust
#[test]
fn drag_preview_reorders_root_children_with_pointer_inside_card_band() {
    let source = "# Root\n## A\n## B\n## C\n";
    let (mut view, doc) = view_with_source(source);
    render_test_view(&mut view, &doc);
    let source_range = node_by_title(view.ready_tree(), "B").subtree_source_range.clone();
    let anchor = view.ready_hit_map().nodes[3].card_rect;

    let response = view.handle_canvas_drag(
        drag_request(CanvasDragPhase::Update, source_range, anchor.x + anchor.w * 0.5, anchor.y),
        &doc,
    );

    assert!(matches!(response, CanvasDragResponse::Preview(CanvasDragPreview { is_valid: true, .. })));
}
```

- [ ] **Step 2: 运行测试，确认当前实现失败**

Run: `cargo test -p textora-markdown drag_preview_reorders_root_children_with_pointer_inside_card_band --lib`

Expected: 测试断言失败，因为现有候选过滤要求 `layout_node.x + layout_node.w <= drag_x`。

- [ ] **Step 3: 写入失败测试，覆盖相邻节点间隙中心插入线**

```rust
#[test]
fn same_level_insertion_line_is_centered_between_neighbor_cards() {
    // 使用 `A` 后、`B` 前的预览；断言 y 等于 `A` 下边缘与 `B` 上边缘的平均值。
}
```

- [ ] **Step 4: 实现最小候选与插入线计算**

```rust
fn same_parent_candidate<'a>(
    layout: &'a LayoutTree,
    tree: &'a Tree,
    source_index: usize,
    preview_rect: Rect,
    drag_x: f32,
    same_level_horizontal_limit: f32,
) -> Option<DragCandidate<'a>> {
    let source = layout.nodes.get(source_index)?;
    (drag_x >= source.x && drag_x <= source.x + source.w + same_level_horizontal_limit)
        .then(|| find_siblings(tree, source_index))
        .flatten()
        .into_iter()
        .flatten()
        .filter(|node_index| *node_index != source_index)
        .filter_map(|node_index| {
            let node = *collect_nodes_dfs(&tree.root).get(node_index)?;
            let layout_node = layout.nodes.get(node_index)?;
            Some(DragCandidate { node_index, node, layout_node })
        })
        .min_by(|left, right| drag_distance(left.layout_node, preview_rect).total_cmp(&drag_distance(right.layout_node, preview_rect)))
}
```

在 `calculate_drag_preview()` 中优先使用 `same_parent_candidate`；无结果时继续调用 `nearest_left_candidate`。使用相邻兄弟的卡片边界生成插入线，首尾位置使用 `sibling_gap * 0.5`。

- [ ] **Step 5: 运行两个测试，确认通过**

Run: `cargo test -p textora-markdown 'drag_preview_reorders_root_children_with_pointer_inside_card_band|same_level_insertion_line_is_centered_between_neighbor_cards' --lib`

Expected: 两项通过。

### Task 2: 统一拖拽渲染反馈

**Files:**

- Modify: `crates/markdown/src/mmf/canvas.rs:518-572,971-1128`

**Interfaces:**

- Consumes: `CanvasDragPreview::insertion_line` 和 `Theme::mindmap.canvas.focus_ring`。
- Produces: 有效拖拽的目标框、导引线和插入线均使用焦点色，插入线高度为 2 像素。

- [ ] **Step 1: 写入失败渲染测试**

```rust
#[test]
fn valid_drag_insertion_feedback_uses_focus_ring_and_two_pixel_line() {
    // 构造带 insertion_line 的 valid_preview。
    // 断言横线 DrawCmd 的 color 是 focus_ring，rect.h 是 2.0。
}
```

- [ ] **Step 2: 运行测试，确认当前实现失败**

Run: `cargo test -p textora-markdown valid_drag_insertion_feedback_uses_focus_ring_and_two_pixel_line --lib`

Expected: 失败，因为当前颜色为 `connector_hover`，高度为 `connector_width * 0.5`。

- [ ] **Step 3: 实现最小渲染调整**

```rust
const DRAG_INSERTION_LINE_HEIGHT: f32 = 2.0;

let color = if preview.is_valid {
    theme.mindmap.canvas.focus_ring
} else {
    theme.mindmap.canvas.drag_invalid
};
```

使用 `Rect::new(from_x + offset_x, from_y + offset_y - DRAG_INSERTION_LINE_HEIGHT * 0.5, to_x - from_x, DRAG_INSERTION_LINE_HEIGHT)` 绘制居中的细线；保留无效拖拽的红色反馈。

- [ ] **Step 4: 运行测试，确认通过**

Run: `cargo test -p textora-markdown valid_drag_insertion_feedback_uses_focus_ring_and_two_pixel_line --lib`

Expected: PASS。

### Task 3: 拖拽手型光标

**Files:**

- Modify: `crates/app/src/events.rs:26-48,730-782`

**Interfaces:**

- Consumes: `MouseState::canvas_drag`、`CanvasDragSession::started` 和 `EditHitTarget::SourceObject`。
- Produces: 已选节点返回 `CursorIcon::Grab`，已启动拖拽会话返回 `CursorIcon::Grabbing`。

- [ ] **Step 1: 将现有光标测试改为失败断言**

```rust
assert_eq!(last_cursor_icon(&handle_cursor_moved(&mut app, pointer.0, pointer.1)), CursorIcon::Grab);
// 设置 started: true 的 canvas_drag 后断言 CursorIcon::Grabbing。
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p textora-app mmap_cursor_reflects_edit_and_drag_targets --lib`

Expected: 失败，因为当前已选节点使用 `CursorIcon::Move`，拖拽过程未优先处理。

- [ ] **Step 3: 实现最小光标状态映射**

```rust
if app.mouse.canvas_drag.as_ref().is_some_and(|session| session.started) {
    return Some(CursorIcon::Grabbing);
}
// SourceObject 已选中时：CursorIcon::Grab
```

- [ ] **Step 4: 运行测试，确认通过**

Run: `cargo test -p textora-app mmap_cursor_reflects_edit_and_drag_targets --lib`

Expected: PASS。

### Task 4: 格式化与回归验证

**Files:**

- Modify: 上述测试和实现文件，仅限 `cargo fmt` 产生的格式化。

- [ ] **Step 1: 格式化**

Run: `cargo fmt --check && cargo fmt`

Expected: 最终 `cargo fmt --check` 无输出且退出码为 0。

- [ ] **Step 2: 运行相关 crate 测试**

Run: `cargo test -p textora-markdown --lib && cargo test -p textora-app --lib`

Expected: 两个命令均退出码为 0。

- [ ] **Step 3: 编译应用 crate**

Run: `cargo check -p textora-app`

Expected: 退出码为 0。

- [ ] **Step 4: 审阅改动范围**

Run: `git diff --check && git status --short`

Expected: 无空白错误，且只包含计划所列三个源码文件、两个文档文件及其测试改动。
