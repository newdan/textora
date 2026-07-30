# 思维导图节点首次直接拖动 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让思维导图节点卡片的非文字区域在首次按下并跨过拖动阈值后直接开始拖放。

**Architecture:** `app` 层启用 `SourceObject` 拖放，统一使用 `CanvasDragEligibility::Enabled` 创建拖动会话；`markdown` 插件层负责标题/IME 按下点保护，画布空白不创建拖动会话。

**Tech Stack:** Rust、`winit` 鼠标事件、`textora-app` 内联单元测试。

## Global Constraints

- `ui` 不得依赖或访问 `app` 层状态；本改动不变更跨层接口。
- 保持现有 5px `PLUGIN_SELECTION_DRAG_THRESHOLD_PX` 阈值。
- 不使用 `.unwrap()`；保持 `cargo fmt`。
- 单元测试必须先观察失败，再写最小实现。

---

### Task 1: 允许未预选的思维导图节点直接开始拖放

**Files:**
- Modify: `crates/app/src/dispatch/mouse.rs:185-218`
- Test: `crates/app/src/dispatch/mouse.rs:859-875`

**Interfaces:**
- Consumes: `EditHitTarget::SourceObject { source_range: Range<usize> }` 与 `CanvasDragEligibility`。
- Produces: `MouseState::canvas_drag`，其 `eligibility` 为 `CanvasDragEligibility::Enabled`，供 `dispatch_canvas_drag_moved` 在超过 5px 后发送 `CanvasDragPhase::Start`。

- [x] **Step 1: 将“首次不可拖动”的测试改为首次可开始拖动的测试**

将测试替换为：

```rust
#[test]
fn mindmap_canvas_drag_starts_without_prior_selection() {
    let state =
        Rc::new(RefCell::new(CanvasDragTestState::with_response(CanvasDragResponse::Ignore)));
    let mut app = app_with_mindmap_drag_plugin("abc", state.clone());
    let bounds = app.plugin_render_bounds();

    app.dispatch_editor_mouse_input(
        ElementState::Pressed,
        bounds.x + 4.0,
        bounds.y + 4.0,
        None,
    );
    app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, None);

    assert_eq!(state.borrow().request_count(CanvasDragPhase::Start), 1);
}
```

- [x] **Step 2: 运行定向测试并确认它因旧资格限制失败**

Run: `cargo test -p textora-app --lib dispatch::mouse::tests::mindmap_canvas_drag_starts_without_prior_selection -- --exact`

Expected: FAIL，断言显示 `CanvasDragPhase::Start` 请求数为 `0` 而非 `1`。

- [x] **Step 3: 移除思维导图的预选资格判断，统一启用 `SourceObject` 拖动**

在 `Some(Some(EditHitTarget::SourceObject { source_range }))` 分支中，删除 `was_selected_before_press` 及条件 `eligibility`，并创建会话时使用：

```rust
self.mouse.canvas_drag = Some(crate::mouse::CanvasDragSession {
    source_range,
    pressed_at: (px, py),
    source_generation,
    eligibility: CanvasDragEligibility::Enabled,
    started: false,
});
```

保留 `apply_edit_hit_target`，使单击仍选中节点；不修改 `TextCaret`、`ClearFocus` 或空白区域分支。

- [x] **Step 4: 运行定向测试并确认通过**

Run: `cargo test -p textora-app --lib dispatch::mouse::tests::mindmap_canvas_drag_starts_without_prior_selection -- --exact`

Expected: PASS，1 个测试通过。

- [x] **Step 5: 格式化并运行相关回归集**

Run: `cargo fmt --check && cargo test -p textora-app --lib dispatch::mouse::tests`

Expected: 两个命令退出码均为 `0`；文字命中不创建拖动会话、拖动阈值与落放事务测试仍通过。

- [x] **Step 6: 运行应用库完整测试**

Run: `cargo test -p textora-app --lib`

Expected: 退出码为 `0`，无失败测试。

- [x] **Step 7: 提交改动**

```bash
git add crates/app/src/dispatch/mouse.rs docs/specs/2026-07-15-mmap-direct-node-drag.md docs/plans/2026-07-15-mmap-direct-node-drag.md
git commit -m "fix(mmap): allow direct node dragging"
```

### Task 2: 防止标题按下启动节点拖放

**Files:**
- Modify: `crates/markdown/src/mindmap_view.rs:411-422,1190-1221`
- Test: `crates/markdown/src/mindmap_view.rs:2104-2128`
- Modify: `docs/specs/2026-07-15-mmap-direct-node-drag.md`

**Interfaces:**
- Consumes: `CanvasDragRequest::{pressed_x, pressed_y, pointer_x, pointer_y}` 与节点标题矩形。
- Produces: 对按下点或当前点位于标题的 `CanvasDragPhase::{Start, Update, Drop}` 请求返回 `CanvasDragResponse::Ignore`，并将 `drag_state` 复位为 `Idle`。

- [x] **Step 1: 写入从标题按下、移至卡片外时仍拒绝拖放的失败测试**

在 `drag_ignores_requests_over_a_title_rect` 附近新增测试：以 `drag_request_at_screen` 构造当前指针位于目标卡片外的 `CanvasDragPhase::Start` 请求，再把 `pressed_x`、`pressed_y` 覆盖为源节点标题矩形中心。断言 `view.handle_canvas_drag` 返回 `CanvasDragResponse::Ignore`。

- [x] **Step 2: 运行该定向测试并确认旧实现错误地返回预览**

Run: `cargo test -p textora-markdown --lib mindmap_view::tests::drag_ignores_requests_started_on_a_title_rect -- --exact`

Expected: FAIL，旧实现在当前指针不位于标题时返回 `CanvasDragResponse::Preview`。

- [x] **Step 3: 让标题命中判断同时检查按下点与当前指针**

将 `drag_request_hits_title` 改为对 `request.pointer_x/request.pointer_y` 和 `request.pressed_x/request.pressed_y` 都转换为内容坐标，并在任一点落入任一 `node.contains_title` 时返回 `true`。保留现有 `Start`、`Update` 与 `Drop` 调用位置，使三阶段共享同一保护规则。

- [x] **Step 4: 运行定向测试并确认通过**

Run: `cargo test -p textora-markdown --lib mindmap_view::tests::drag_ignores_requests_started_on_a_title_rect -- --exact`

Expected: PASS，1 个测试通过。

- [x] **Step 5: 格式化并运行 markdown 与应用回归集**

Run: `cargo fmt --check && cargo test -p textora-markdown --lib mindmap_view::tests && cargo test -p textora-app --lib dispatch::mouse::tests`

Expected: 三个命令退出码均为 `0`；既有标题落点拒绝、首次直接拖动与文字命中测试均通过。

- [x] **Step 6: 运行应用库完整测试并提交修复**

Run: `cargo test -p textora-app --lib`

Expected: 退出码为 `0`，无失败测试。

```bash
git add crates/markdown/src/mindmap_view.rs docs/specs/2026-07-15-mmap-direct-node-drag.md docs/plans/2026-07-15-mmap-direct-node-drag.md
git commit -m "fix(mmap): reject drags started on titles"
```
