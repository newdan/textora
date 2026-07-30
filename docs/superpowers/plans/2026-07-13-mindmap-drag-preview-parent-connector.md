# Mindmap 拖拽预览父节点连接线 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在所有合法的思维导图拖拽预览位置显示从真实目标父节点到预览卡片的临时连接线，同时保留排序插入线。

**Architecture:** `MindmapView` 在计算拖拽预览时依据最终移动目标确定真实父节点，并把其卡片右侧中点写入 `CanvasDragPreview::guide_to`。`mmf::canvas` 保持为纯渲染消费者，继续并行绘制 `guide_to` 的连接线和 `insertion_line` 的插入线。

**Tech Stack:** Rust、`textora-markdown`、内联单元测试。

## Global Constraints

- 不让 `ui` 访问 Markdown 或应用层状态。
- 不改变落点选择、移动事务、源子树透明度、颜色或连接线路径。
- 根节点仍不可拖动。
- 所有生产改动先有失败测试覆盖；完成前运行 `cargo fmt` 和相关 crate 测试。

---

### Task 1: 目标父节点导引线

**Files:**
- Modify: `crates/markdown/src/mindmap_view.rs:444-514,1810-1865`

**Interfaces:**
- Consumes: `find_parent(&Tree, usize) -> Option<&Node>`、DFS 节点列表、`LayoutTree`、`MoveSubtreeTarget`。
- Produces: 合法 `CanvasDragPreview` 的 `guide_to: Some((parent_right, parent_center_y))`；排序目标仍产生 `insertion_line`。

- [ ] **Step 1: 写入失败测试**

在 `drag_preview_reorders_root_children_with_pointer_inside_card_band` 后新增测试，令三级节点 `Source` 拖至另一个二级节点 `Target` 的直接子节点 `B` 前。断言预览合法、目标为 `BeforeSibling`、插入线存在，并且 `guide_to` 等于 `Target` 卡片右侧中点：

```rust
#[test]
fn cross_level_sibling_preview_connects_to_the_target_parent() {
    let source = "# Root\n## SourceParent\n### Source\n## Target\n### A\n### B\n";
    let (mut view, doc) = view_with_source(source);
    render_test_view(&mut view, &doc);
    let source_range = node_by_title(view.ready_tree(), "Source").subtree_source_range.clone();
    let target_parent = view.ready_hit_map().nodes[3].card_rect;
    let anchor = view.ready_hit_map().nodes[5].card_rect;

    let response = view.handle_canvas_drag(
        drag_request(&view, CanvasDragPhase::Update, source_range, anchor.x + 1.0, anchor.y, 1),
        &doc,
    );

    let CanvasDragResponse::Preview(preview) = response else {
        panic!("cross-level sibling target should produce a preview");
    };
    assert!(preview.is_valid);
    assert_eq!(preview.guide_to, Some((target_parent.x + target_parent.w, target_parent.y + target_parent.h * 0.5)));
    assert!(preview.insertion_line.is_some());
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p textora-markdown cross_level_sibling_preview_connects_to_the_target_parent --lib`

Expected: 失败，`preview.guide_to` 为 `None`。

- [ ] **Step 3: 实现最小父节点解析**

在 `calculate_drag_preview()` 中，在 `candidate_target` 确定后计算父节点索引：同级目标使用锚点的父节点；子节点目标使用锚点本身。通过 DFS 节点列表定位父节点索引并读取布局，统一写入 `canvas.guide_to`：

```rust
let target_parent_index = match candidate_target {
    mmf::edit::MoveSubtreeTarget::BeforeSibling
    | mmf::edit::MoveSubtreeTarget::AfterSibling => find_parent(tree, anchor_index)
        .and_then(|parent| nodes.iter().position(|node| std::ptr::eq(*node, parent))),
    mmf::edit::MoveSubtreeTarget::BeforeChild
    | mmf::edit::MoveSubtreeTarget::LastChild => Some(anchor_index),
};
if let Some(parent) = target_parent_index.and_then(|index| layout.nodes.get(index)) {
    canvas.guide_to = Some((parent.x + parent.w, parent.y + parent.h * 0.5));
}
```

删除只对 `BeforeChild` / `LastChild` 设置 `guide_to` 的旧分支。

- [ ] **Step 4: 运行目标测试，确认通过**

Run: `cargo test -p textora-markdown cross_level_sibling_preview_connects_to_the_target_parent --lib`

Expected: PASS。

- [ ] **Step 5: 运行拖拽预览回归测试**

Run: `cargo test -p textora-markdown drag_preview --lib`

Expected: PASS；将已有同级测试的 `guide_to == None` 断言更新为根节点右侧中点。

- [ ] **Step 6: 格式化并提交**

Run: `cargo fmt --check`

Expected: PASS。

Commit:

```bash
git add crates/markdown/src/mindmap_view.rs
git commit -m "fix(mmap): connect drag previews to target parents"
```

### Task 2: Crate 级验证

**Files:**
- Verify: `crates/markdown/src/mindmap_view.rs`

**Interfaces:**
- Consumes: Task 1 的拖拽预览几何。
- Produces: 已验证的 `textora-markdown` crate。

- [ ] **Step 1: 运行完整 Markdown 单元测试**

Run: `cargo test -p textora-markdown --lib`

Expected: PASS，所有测试通过。

- [ ] **Step 2: 运行编译检查**

Run: `cargo check -p textora-markdown`

Expected: PASS，无编译错误。
