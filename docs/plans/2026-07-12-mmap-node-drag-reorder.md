# mmap Node Drag Reorder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 `.mmap.md` 提供清晰的整节点选中态，以及基于“左侧最近锚点”的拖拽排序和父子层级调整，并以一次原子源码事务完成每次移动。

**Architecture:** `ui::plugin` 新增不认识 MMF 的画布拖放请求/响应；`app` 只保存通用拖放会话、转发鼠标阶段并执行已有 `EditTransaction`；`textora-markdown` 基于解析树和布局计算锚点、预览与局部源码 replacement。拖放的唯一真实状态仍是 `DocumentView` 中的源码，插件内的拖拽预览可随时丢弃。

**Tech Stack:** Rust、`textora-ui` 插件协议与主题、`textora-app` 鼠标分发/事务、`textora-markdown` MMF 解析树、布局、canvas 和单元测试。

## Global Constraints

- 产品名使用 `textora`，Markdown crate 包名使用 `textora-markdown`。
- `ui` 只能定义纯数据协议和视觉主题；不得依赖 `DocumentView`、`Workspace` 或 app 状态。
- app 不解析 MMF、不保存节点索引；MMF 语义和源码移动计划只位于 `textora-markdown`。
- 根节点固定：不可拖动，也绝不作为同级排序锚点。
- 标题区域继续处理光标与文本选择；仅卡片非标题区域可启动节点拖拽。
- 复用 `PLUGIN_SELECTION_DRAG_THRESHOLD_PX` 的 `5px` 阈值，避免点击变拖拽。
- 横向同级阈值为 `level_indent × 0.35`；超过该阈值即为目标最后一个子节点。
- 任何移动都只生成一个 `EditTransaction`，并在成功后选中移动后的完整子树。
- 禁止 `.unwrap()`；不可恢复的不变量使用带原因的 `.expect("...")`。
- 每个任务先写失败测试，再写最小实现；每次提交前执行 `cargo fmt` 和该任务的编译/测试命令。

---

## 文件与职责

| 文件 | 职责 |
| --- | --- |
| `crates/ui/src/plugin.rs` | 画布拖放的通用请求、预览、响应及 `ViewPlugin` 默认入口。 |
| `crates/ui/src/theme/mindmap.rs` | 选中、拖动预览、无效落点和几何常量的默认主题。 |
| `crates/ui/src/theme_file.rs` | 将新增 mindmap 主题字段暴露给 TOML 配置文件。 |
| `crates/markdown/src/mmf/edit.rs` | 以两个非重叠 replacement 生成子树移动事务，并计算移动后的选择范围。 |
| `crates/markdown/src/mindmap_view.rs` | 将 MMF 树/布局映射为锚点、落点、拖放预览和最终事务。 |
| `crates/markdown/src/mmf/canvas.rs` | 绘制明显的节点选中态、拖动投影、引导线和插入标记。 |
| `crates/app/src/mouse.rs` | 保存不包含 MMF 语义的自绘插件拖放会话。 |
| `crates/app/src/dispatch/mouse.rs` | 把按下、跨阈值移动、释放和取消转发给自绘插件并执行返回的事务。 |

## Task 1: 定义通用画布拖放协议

**Files:**

- Modify: `crates/ui/src/plugin.rs:1-470`
- Test: `crates/ui/src/plugin.rs` 内的 `#[cfg(test)]` 模块

**Interfaces:**

- Produces: `CanvasDragPhase::{Start, Update, Drop, Cancel}`。
- Produces: `CanvasDragRequest { phase, source_range, pointer_x, pointer_y, offset_x, offset_y, source_generation }`。
- Produces: `CanvasDragPreview { source_rect, preview_rect, guide_from, guide_to, insertion_line, target_rect, is_valid }`，其中所有矩形和点都是屏幕像素的纯 UI 数据。
- Produces: `CanvasDragResponse::{Ignore, Preview(CanvasDragPreview), Apply(EditTransaction)}`。
- Produces: `ViewPlugin::handle_canvas_drag(&mut self, request, doc) -> CanvasDragResponse`，默认 `Ignore`。
- Consumes: 已有 `EditTransaction`、`Rect` 与 `DocView`。

- [ ] **Step 1: 写失败测试，锁定默认插件不会处理拖放**

在 `plugin.rs` 测试模块中为现有 `StubPlugin` 调用默认入口；测试请求使用完整且通用的源码范围：

```rust
#[test]
fn view_plugin_ignores_canvas_drag_by_default() {
    let mut plugin = StubPlugin { name: "stub" };
    let response = plugin.handle_canvas_drag(
        CanvasDragRequest {
            phase: CanvasDragPhase::Start,
            source_range: 3..8,
            pointer_x: 40.0,
            pointer_y: 60.0,
            offset_x: 0.0,
            offset_y: 0.0,
            source_generation: 7,
        },
        &TestDoc::new("abcdefghi"),
    );
    assert!(matches!(response, CanvasDragResponse::Ignore));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-ui --lib view_plugin_ignores_canvas_drag_by_default`

Expected: FAIL，错误指出 `CanvasDragRequest`、`CanvasDragPhase` 和 `handle_canvas_drag` 尚未定义。

- [ ] **Step 3: 在 `ui::plugin` 实现纯数据协议和默认入口**

在 `EditPlan` 定义之后加入以下类型；`CanvasDragPreview` 不含节点 ID、树索引或 markdown 类型：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasDragPhase {
    Start,
    Update,
    Drop,
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanvasDragRequest {
    pub phase: CanvasDragPhase,
    pub source_range: std::ops::Range<usize>,
    pub pointer_x: f32,
    pub pointer_y: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub source_generation: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanvasDragPreview {
    pub source_rect: Rect,
    pub preview_rect: Rect,
    pub guide_from: (f32, f32),
    pub guide_to: Option<(f32, f32)>,
    pub insertion_line: Option<((f32, f32), (f32, f32))>,
    pub target_rect: Option<Rect>,
    pub is_valid: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CanvasDragResponse {
    Ignore,
    Preview(CanvasDragPreview),
    Apply(EditTransaction),
}
```

在 `ViewPlugin` 中添加默认实现：

```rust
fn handle_canvas_drag(
    &mut self,
    _request: CanvasDragRequest,
    _doc: &dyn DocView,
) -> CanvasDragResponse {
    CanvasDragResponse::Ignore
}
```

- [ ] **Step 4: 运行测试确认通过并格式化**

Run: `cargo fmt --check && cargo test -p textora-ui --lib view_plugin_ignores_canvas_drag_by_default`

Expected: PASS。

- [ ] **Step 5: 提交协议边界**

```bash
git add crates/ui/src/plugin.rs
git commit -m "feat(ui): add generic canvas drag protocol"
```

## Task 2: 让主题完整表达选中与拖拽反馈

**Files:**

- Modify: `crates/ui/src/theme/mindmap.rs:1-350`
- Modify: `crates/ui/src/theme_file.rs:115-510`
- Test: `crates/ui/src/theme/mindmap.rs` 与 `crates/ui/src/theme_file.rs` 内的测试模块

**Interfaces:**

- Produces: `MindmapCanvasTheme::drag_invalid` 与默认深浅色值。
- Produces: `MindmapGeometry::{selection_outline_width, selection_outline_gap, drag_source_alpha, drag_preview_alpha, same_level_threshold_ratio}`。
- Produces: 对应 `MindmapCanvasFile`、`MindmapGeometryFile` 的可选 TOML 字段和 `resolve_mindmap` 映射。
- Consumes: Task 1 的通用预览只使用这些主题值，不嵌入硬编码尺寸或透明度。

- [ ] **Step 1: 写失败测试，锁定主题默认值和 TOML 覆盖**

加入两个测试：一个断言深浅主题的 `drag_invalid` 具有非零 alpha、`selection_outline_width > 1.0` 且 `same_level_threshold_ratio == 0.35`；另一个加载：

```toml
[mindmap.canvas]
drag_invalid = "#D94A4AFF"

[mindmap.geometry]
selection_outline_width = 3.0
same_level_threshold_ratio = 0.35
```

并断言最终 `Theme` 使用这三个覆盖值。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-ui --lib mindmap`

Expected: FAIL，新增字段尚不存在。

- [ ] **Step 3: 添加主题字段、gamma 校正和 TOML 解析**

扩展 canvas/geometry 定义、`gamma_correct()`、`default_dark()`、`default_light()`、主题文件可选结构和 `resolve_mindmap()`。保持参数语义如下：

```rust
pub drag_invalid: [f32; 4],

pub selection_outline_width: f32,
pub selection_outline_gap: f32,
pub drag_source_alpha: f32,
pub drag_preview_alpha: f32,
pub same_level_threshold_ratio: f32,
```

不要把 `same_level_threshold_ratio` 放进 `canvas`；它是与 DPI 无关的布局比例。`selection_outline_width` 与 `selection_outline_gap` 在 `MindmapView::render()` 中随 `dpi_scale` 进入 canvas 常量。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo fmt --check && cargo test -p textora-ui --lib mindmap && cargo check -p textora-ui`

Expected: PASS。

- [ ] **Step 5: 提交主题能力**

```bash
git add crates/ui/src/theme/mindmap.rs crates/ui/src/theme_file.rs
git commit -m "feat(ui): theme mmap drag feedback"
```

## Task 3: 以局部事务移动 MMF 子树

**Files:**

- Modify: `crates/markdown/src/mmf/edit.rs:1-620`
- Test: `crates/markdown/src/mmf/edit.rs` 内的 `#[cfg(test)]` 模块

**Interfaces:**

- Produces: `MoveSubtreeTarget::{BeforeSibling, AfterSibling, LastChild}`。
- Produces: `plan_move_subtree(tree, source, source_range, anchor_range, target, source_generation) -> EditPlan`。
- Produces: 一笔 `EditTransaction`，其中最多两个非重叠 replacement：删除源子树、在原始坐标插入层级已调整的子树文本。
- Consumes: `Node::{subtree_source_range, heading_marker_range, heading_level}` 与 `find_parent` / `find_siblings`。

- [ ] **Step 1: 写失败测试，覆盖排序、挂子级与拒绝规则**

在 `edit.rs` 的现有树 fixture 旁加入以下测试，并统一用 `apply_transaction_to_text()` 辅助函数按 replacement 起点逆序应用事务：

```rust
#[test]
fn move_subtree_after_sibling_preserves_nested_content_and_selects_it() {
    let source = "# Root\n## A\nA note\n### A1\n## B\n";
    let tree = parse(source).expect("fixture must be valid MMF");
    let plan = plan_move_subtree(
        &tree,
        source,
        node_range(&tree, "A"),
        node_range(&tree, "B"),
        MoveSubtreeTarget::AfterSibling,
        4,
    );
    assert_transaction_text_and_selection(
        plan,
        source,
        "# Root\n## B\n## A\nA note\n### A1\n",
        "## A\nA note\n### A1\n",
    );
}

#[test]
fn move_subtree_as_last_child_increases_every_heading_level() {
    let source = "# Root\n## A\n### A1\n## B\n";
    let tree = parse(source).expect("fixture must be valid MMF");
    let plan = plan_move_subtree(
        &tree,
        source,
        node_range(&tree, "A"),
        node_range(&tree, "B"),
        MoveSubtreeTarget::LastChild,
        9,
    );
    assert_transaction_text_and_selection(
        plan,
        source,
        "# Root\n## B\n### A\n#### A1\n",
        "### A\n#### A1\n",
    );
}
```

再补充：源节点在锚点之后的前置排序、跨父同级排序被拒绝、根节点移动被拒绝、锚点是源后代被拒绝、CRLF 文档保留 `\r\n`、属性/代码围栏备注原样保留的断言。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-markdown --lib mmf::edit::tests::move_subtree_after_sibling_preserves_nested_content_and_selects_it`

Expected: FAIL，`MoveSubtreeTarget` 与 `plan_move_subtree` 尚未定义。

- [ ] **Step 3: 实现仅依赖树和源码的移动计划器**

实现下列流程，函数均为私有 helper，只有 `MoveSubtreeTarget` 和 `plan_move_subtree` 对 `mindmap_view` 可见：

1. 用 `subtree_source_range` 精确定位源和锚点；源必须不是 DFS 索引 `0`。
2. 用指针比较排除源本身与 `collect_nodes_dfs(source_node)` 中的任何节点。
3. `BeforeSibling` / `AfterSibling` 必须共享同一父节点，且锚点不可为根；`LastChild` 允许根锚点。
4. 读取完整源块，按 `anchor.heading_level + sibling_offset` 计算目标根层级；`sibling_offset` 为 `0`，`LastChild` 为 `1`。
5. 对源子树每个 `heading_marker_range` 相对源块起点的位置插入或删除 `#`，使每个后代与源根保持原相对深度。
6. 目标插入位置分别是锚点 `subtree_source_range.start`、`subtree_source_range.end`、`subtree_source_range.end`。用 `ensure_block_boundaries()` 保证移动块与相邻标题之间恰有文档既有换行风格的分隔。
7. 返回删除源范围和插入范围两个 replacement；它们在原始坐标中不重叠，现有执行器的逆序写入可正确处理源在锚点前后两种情况。
8. 计算移动后范围：插入点在源前时为 `insertion..insertion + moved_len`，在源后时为 `insertion - source_len .. insertion - source_len + moved_len`；将它写入 `EditSelection::Range`。

无效目标一律返回 `EditPlan::Consume`，不得产生部分 replacement。

- [ ] **Step 4: 运行相关测试确认通过**

Run: `cargo fmt --check && cargo test -p textora-markdown --lib mmf::edit::tests && cargo check -p textora-markdown`

Expected: PASS。

- [ ] **Step 5: 提交源码移动器**

```bash
git add crates/markdown/src/mmf/edit.rs
git commit -m "feat(markdown): plan mmap subtree moves"
```

## Task 4: 计算 mmap 拖放预览并绘制清晰反馈

**Files:**

- Modify: `crates/markdown/src/mindmap_view.rs:1-880`
- Modify: `crates/markdown/src/mmf/canvas.rs:1-850`
- Test: 两个文件各自的 `#[cfg(test)]` 模块

**Interfaces:**

- Consumes: Task 1 的 `CanvasDragRequest/Response/Preview`、Task 2 的主题字段、Task 3 的 `plan_move_subtree`。
- Produces: `MindmapDragState::{Idle, Preview(MindmapDragPreview)}`，其中预览只保存 `source_range`、候选语义和 `CanvasDragPreview`。
- Produces: `MindmapView::handle_canvas_drag()`：`Start/Update` 返回预览，`Drop` 返回事务或 `Ignore`，`Cancel` 清除状态。
- Produces: canvas 中的选中增强层、拖动源弱化、投影卡片、左向引导线、同级插入线、子级目标强调和无效色。

- [ ] **Step 1: 写失败测试，锁定命中、候选和渲染结果**

在 `mindmap_view.rs` 用既有 `view_with_source()` 与 `render_test_view()` helper 添加：

```rust
#[test]
fn drag_preview_uses_nearest_left_node_and_same_level_after_marker() {
    let source = "# Root\n## A\n## B\n## C\n";
    let (mut view, doc) = view_with_source(source);
    render_test_view(&mut view, &doc);
    let source_range = node_by_title(view.ready_tree(), "B").subtree_source_range.clone();
    let response = view.handle_canvas_drag(drag_request(
        CanvasDragPhase::Update,
        source_range,
        240.0,
        120.0,
        1,
    ), &doc);
    assert_preview_has_same_level_insertion(response, "C", InsertionSide::After);
}
```

再加入以下断言：标题矩形中的请求返回 `Ignore`；源为根、锚点为源后代和 generation 不一致只返回 `Preview` 且 `is_valid == false` 或直接 `Ignore`；在不同横向层级时得到 `LastChild`；`Drop` 的 `Apply` 生成 Task 3 已锁定的结果；`Cancel` 与 `UpdateSource` 后不再渲染预览。

在 `canvas.rs` 扩展现有 `node_selection_uses_focus_ring_without_drawing_a_caret`：断言选中节点有 `selection` 填充与至少两个 `focus_ring` 描边，且最外层描边宽度等于主题的 `selection_outline_width`。新增拖拽测试断言合法同级预览有投影填充、引导线和插入线，无效预览使用 `drag_invalid` 且没有插入线。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-markdown --lib drag_preview_uses_nearest_left_node_and_same_level_after_marker && cargo test -p textora-markdown --lib node_selection_uses_focus_ring_without_drawing_a_caret`

Expected: FAIL，`handle_canvas_drag` 覆盖和新主题化绘制尚未实现。

- [ ] **Step 3: 在 `MindmapView` 实现拖拽状态与候选计算**

在 `MindmapView` 内新增互斥 `MindmapDragState`；`UpdateSource`、`ClearEditFocus`、无效文档状态和 `CanvasDragPhase::Cancel` 全部回到 `Idle`。候选计算必须：

```rust
let drag_x = request.pointer_x - request.offset_x;
let drag_y = request.pointer_y - request.offset_y + self.scroll_y;
let candidate = nearest_left_candidate(layout, tree, source_node, drag_x, drag_y)?;
let same_level = (drag_x - candidate.layout_node.x).abs()
    <= self.constants.level_indent * theme_ratio;
```

`nearest_left_candidate` 只接受候选卡片右边缘不超过 `drag_x` 的节点；按“距离、DFS 索引”稳定排序。选择同级时用 `drag_y < candidate.center_y()` 区分 `BeforeSibling`/`AfterSibling`；否则生成 `LastChild`。调用 `mmf::edit::plan_move_subtree()` 前再次校验 candidate，避免过期预览提交。

`MindmapRenderProjection` 增加可选拖拽预览引用；布局仍使用原树，不让预览触发解析、重排或修改命中几何。

- [ ] **Step 4: 在 canvas 按固定顺序渲染状态**

更新 `render_cards_and_connectors()`，绘制顺序固定为：普通连接线、普通卡片、选中增强层、拖动源弱化、拖动投影、引导线/目标强调/插入线、标题选择、文本、IME 下划线、光标。新增小型纯函数：

```rust
fn with_alpha(color: [f32; 4], alpha: f32) -> [f32; 4] {
    [color[0], color[1], color[2], color[3] * alpha]
}
```

同级插入线用 `DrawList::fill_rounded()` 的窄矩形实现；引导线用已有分段 connector 绘制算法，不修改 `DrawCmd`。投影卡片使用源节点的解析后样式和 `drag_preview_alpha`，不渲染第二份标题文本以免与指针附近的编辑语义混淆。

- [ ] **Step 5: 运行 markdown 全量单元测试**

Run: `cargo fmt --check && cargo test -p textora-markdown --lib && cargo check -p textora-markdown`

Expected: PASS。

- [ ] **Step 6: 提交视图与绘制实现**

```bash
git add crates/markdown/src/mindmap_view.rs crates/markdown/src/mmf/canvas.rs
git commit -m "feat(markdown): preview mmap node drag moves"
```

## Task 5: 在 app 中接入通用拖放生命周期

**Files:**

- Modify: `crates/app/src/mouse.rs:1-65`
- Modify: `crates/app/src/dispatch/mouse.rs:1-540`
- Test: `crates/app/src/dispatch/mouse.rs` 内的 `#[cfg(test)]` 模块

**Interfaces:**

- Consumes: Task 1 的通用拖放协议和 Task 4 的 mmap 响应；不检查插件名称。
- Produces: `MouseState::canvas_drag: Option<CanvasDragSession>`，只保存 `source_range`、按下坐标、源码 generation 和 `started`。
- Produces: `App::dispatch_canvas_drag()`，将阶段转发、渲染预览、在 `Apply` 时调用既有 `execute_edit_plan()`，随后同步插件源码/光标/选择。

- [ ] **Step 1: 写失败集成测试，锁定 app 只在有效释放时改文档**

复用 `semantic_test_support::app_with_semantic_plugin`，让桩插件记录 `CanvasDragRequest` 并对不同阶段返回响应。新增测试：

```rust
#[test]
fn canvas_drag_starts_after_threshold_and_applies_one_drop_transaction() {
    let state = Rc::new(RefCell::new(SemanticPluginState::with_canvas_drag_response(
        CanvasDragResponse::Apply(EditTransaction::replace(0, 1..2, "Z".into(), 2)),
    )));
    let mut app = app_with_semantic_plugin("abc", state.clone());
    let bounds = app.plugin_render_bounds();

    app.dispatch_editor_mouse_input(ElementState::Pressed, bounds.x + 4.0, bounds.y + 4.0, None);
    app.dispatch_editor_cursor_moved(bounds.x + 6.0, bounds.y + 6.0, None);
    assert!(state.borrow().canvas_drag_requests.is_empty());

    app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, None);
    app.dispatch_editor_mouse_input(ElementState::Released, bounds.x + 20.0, bounds.y + 20.0, None);

    assert_eq!(active_text(&app), "aZc");
    assert_eq!(state.borrow().drop_request_count(), 1);
}
```

补充测试：`TextCaret` 按下永不建立 `canvas_drag`；插件返回 `Ignore` 时不改文档；取消会发送 `Cancel` 且不会执行事务；过期 generation 返回的事务被既有校验拒绝；鼠标释放后会话清理。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-app --lib dispatch::mouse::tests::canvas_drag_starts_after_threshold_and_applies_one_drop_transaction`

Expected: FAIL，`canvas_drag` 会话和请求记录尚未存在。

- [ ] **Step 3: 增加通用会话并接线按下/移动/释放**

在 `mouse.rs` 定义：

```rust
pub(crate) struct CanvasDragSession {
    pub source_range: Range<usize>,
    pub pressed_at: (f32, f32),
    pub source_generation: u32,
    pub started: bool,
}
```

在自绘插件的按下分支中，仅当 `HitTestEditTarget` 返回 `SourceObject` 时在应用其选中效果后建立会话；保留标题的既有文本选择路径。移动时先比较 `pressed_at` 与当前坐标；达到 `PLUGIN_SELECTION_DRAG_THRESHOLD_PX` 后发送 `Start`，随后每次移动发送 `Update`。收到 `Preview` 只请求重绘；收到 `Ignore` 保持文档不变。

释放时，已启动的会话发送 `Drop`。只对 `CanvasDragResponse::Apply(transaction)` 调用：

```rust
let outcome = execute_edit_plan(EditPlan::Apply(transaction), &mut tab.doc, &[])?;
sync_plugin_after_transaction(tab, &outcome);
```

沿用现有错误到 `AppEffect::REDRAW` 的处理方式，不能用 `?` 跨越事件处理函数；事务验证失败时清理会话、同步原状态、返回重绘。失焦/标签切换路径调用同一 `cancel_canvas_drag()` helper，确保对活跃插件仅发送一次 `Cancel`。

- [ ] **Step 4: 运行 app 定向及回归测试**

Run: `cargo fmt --check && cargo test -p textora-app --lib -- mouse && cargo test -p textora-app --lib -- mmap_ && cargo check -p textora-app`

Expected: PASS；既有文本拖选仍通过。

- [ ] **Step 5: 提交 app 接线**

```bash
git add crates/app/src/mouse.rs crates/app/src/dispatch/mouse.rs
git commit -m "feat(app): dispatch canvas node drag transactions"
```

## Task 6: 端到端验证与手工验收

**Files:**

- Modify: `crates/app/src/app_tests.rs:已有 mmap 测试附近`
- Modify: `test_data/sample.mmap.md`（仅在缺少拖动排序、跨父子树和属性/备注 fixture 时）
- Test: `crates/app/src/app_tests.rs`

**Interfaces:**

- Consumes: Tasks 1-5 的最终协议与真实 `MindmapView`。
- Produces: 对真实 mmap 插件的拖放、选择、Undo/Redo 和取消回归保护。

- [ ] **Step 1: 写真实 mmap 端到端失败测试**

在现有 `app_with_mmap_source()` helper 旁加入两项测试：

```rust
#[test]
fn mmap_drag_reorders_siblings_and_undo_restores_the_original_source() {
    let source = "# Root\n## A\n## B\n## C\n";
    let mut app = app_with_mmap_source(source);
    drag_mmap_node(&mut app, "B", DragTarget::After("C"));
    assert_eq!(active_text(&app), "# Root\n## A\n## C\n## B\n");
    undo_active_mmap_edit(&mut app);
    assert_eq!(active_text(&app), source);
}

#[test]
fn mmap_drag_to_left_anchor_makes_the_source_a_child_with_its_subtree() {
    let source = "# Root\n## A\n### A1\n## B\n";
    let mut app = app_with_mmap_source(source);
    drag_mmap_node(&mut app, "A", DragTarget::LastChildOf("B"));
    assert_eq!(active_text(&app), "# Root\n## B\n### A\n#### A1\n");
}
```

`drag_mmap_node()` 必须通过真实 `dispatch_editor_mouse_input` / `dispatch_editor_cursor_moved` 坐标驱动，不可直接调用 `plan_move_subtree()`；另加根节点拖动和标题文本拖选不移动节点的回归测试。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-app --lib -- mmap_drag_`

Expected: 在 Tasks 1-5 完成前 FAIL；完成后 PASS。

- [ ] **Step 3: 补充 fixture 与端到端断言**

仅在 `test_data/sample.mmap.md` 缺少三层节点、属性 TOML 与多行备注样例时添加最小样例。断言拖放后：完整源码、移动节点的选择范围、Undo、Redo 和保存 dirty 状态均正确。

- [ ] **Step 4: 运行完整验证**

Run: `cargo fmt --check && cargo test -p textora-markdown --lib && cargo test -p textora-app --lib && cargo check -p textora-app && ./scripts/verify.sh`

Expected: 所有命令 PASS。

- [ ] **Step 5: 提交验证与 fixture**

```bash
git add crates/app/src/app_tests.rs test_data/sample.mmap.md
git commit -m "test(app): cover mmap node drag reorder"
```

## 计划自检

- 设计中的每项已确认规则均有覆盖：明确选中态（Task 2/4）、左侧最近锚点与 35% 阈值（Task 4）、同级前后排序（Task 3/4）、子级移动（Task 3/4）、根节点固定与环路拒绝（Task 3/4）、单步撤销（Task 3/5/6）。
- 没有占位内容、模糊的异常处理描述或未定义接口；所有新增跨任务类型均在 Task 1-3 中先定义。
- 每个任务修改不超过三个文件，并有独立失败测试、通过测试和提交检查点。
