# mmap 拖拽目标匹配与子树置灰实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 以被拖 mmap 节点的左边界匹配候选节点右边界，并在拖拽时将源子树完整置灰。

**Architecture:** `textora-markdown` 从 `preview_rect.left` 为所有合法候选统一评分，按候选 `right` 决定同级或子级语义。`ui::plugin::CanvasDragPreview` 只增加源子树矩形的纯数据投影；`mmf::canvas` 根据这些矩形对同一子树的连接线、卡片、文字和选中框使用既有 `drag_source_alpha`。

**Tech Stack:** Rust、textora-markdown、textora-ui、cargo test、cargo fmt。

## 全局约束

- `ui` 不得依赖 `markdown` 或 `app` 的状态类型。
- 使用 `CanvasDragPreview` 的纯 `Rect` 输入表达源子树，不传递 mmap 节点 ID 或树引用。
- 禁止 `.unwrap()`；测试中的不可失败前提使用带原因的 `.expect(...)`。
- 修改完成前执行 `cargo fmt --check`、markdown 定向测试、ui 定向测试、`cargo check -p textora-markdown`，以及 `./scripts/verify.sh`。

---

### Task 1: 统一拖拽候选的 left/right 匹配

**Files:**

- Modify: `crates/markdown/src/mindmap_view.rs:348-454, 721-849, 1480-2050`

**Interfaces:**

- Consumes: `LayoutTree::nodes` 的 `x/y/w/h` 与 DFS 顺序。
- Produces: `nearest_drag_candidate(layout, tree, source_node, source_index, preview_rect)`，返回不含自身/后代、且根节点只在 `preview_rect.x > root.right` 时合法的 `DragCandidate`。

- [ ] **Step 1: 写失败测试，锁定抓取点不影响候选与层级**

  在 `mindmap_view.rs` 的测试模块添加测试，构造 `# Root / ## A / ## B / ### B1 / ## C`。对同一个 `preview_rect.x`，分别以源节点中点与右侧作为 `pointer_x`（通过相应 `offset_x` 保持 `preview_rect.x` 相同），断言两次预览均锚定 `B` 并使用 `BeforeSibling`。再将 `preview_rect.x` 移至 `B.right` 之后，断言锚定 `B1` 且使用 `BeforeChild`。

  ```rust
  #[test]
  fn drag_target_uses_preview_left_not_pointer_grab_position() {
      // 两个 request 的 pointer_x 不同，但 pointer_x - offset_x 相同。
      // 二者必须产生相同 anchor_range 和 MoveSubtreeTarget。
  }
  ```

- [ ] **Step 2: 运行测试，确认它因旧的 `drag_x` 筛选失败**

  Run: `cargo test -p textora-markdown --lib drag_target_uses_preview_left_not_pointer_grab_position`

  Expected: FAIL，两个不同抓取点会选到不同候选或目标类型。

- [ ] **Step 3: 以一个评分器替换四段候选回退链**

  删除 `candidate_at_pointer`、`nearest_left_candidate`、`nearest_same_level_candidate` 与 `root_candidate`；新增以下接口，并让 `calculate_drag_preview` 只调用它：

  ```rust
  fn nearest_drag_candidate<'a>(
      layout: &'a LayoutTree,
      tree: &'a Tree,
      source_node: &'a mmf::Node,
      source_index: usize,
      preview_rect: Rect,
  ) -> Option<DragCandidate<'a>> {
      // 排除 source subtree；根仅在 preview_rect.x > root.right 时保留。
      // 按 drag_distance(layout_node, preview_rect)，再按 DFS index 排序。
  }
  ```

  `drag_distance` 继续使用 `preview_rect.x - node.right` 和两个卡片的垂直中心。候选语义只使用：

  ```rust
  let same_level = preview_rect.x <= candidate.layout_node.x + candidate.layout_node.w;
  ```

  根据 `same_level` 选择 `BeforeSibling`/`AfterSibling` 或 `BeforeChild`/`LastChild`，保留现有 generation、根、后代与空操作校验。

- [ ] **Step 4: 运行新增测试与既有拖拽测试**

  Run: `cargo test -p textora-markdown --lib drag_target_uses_preview_left_not_pointer_grab_position`

  Expected: PASS。

  Run: `cargo test -p textora-markdown --lib drag_`

  Expected: PASS，既有同级、子级、根级与后代排除测试全部通过。

- [ ] **Step 5: 提交候选计算实现**

  ```bash
  git add crates/markdown/src/mindmap_view.rs
  git commit -m "fix(markdown): match mmap drag targets by card edge"
  ```

### Task 2: 将源子树几何投影到纯 UI 协议

**Files:**

- Modify: `crates/ui/src/plugin.rs:412-421`
- Modify: `crates/markdown/src/mindmap_view.rs:393-402`

**Interfaces:**

- Produces: `CanvasDragPreview` 为每个拖拽预览包含源节点与后代矩形。

- [ ] **Step 1: 写失败的预览投影测试**

  在 `mindmap_view.rs` 添加一个含 `Source` 和 `SourceChild` 的树。更新拖拽预览后，断言 `CanvasDragResponse::Preview` 的 `source_subtree_rects` 恰好等于这两个节点的卡片矩形，且不包含根或同级节点。

  ```rust
  #[test]
  fn drag_preview_projects_only_the_source_subtree_rectangles() {
      // source_subtree_rects == [source_card, source_child_card]
  }
  ```

- [ ] **Step 2: 运行测试，确认字段尚不存在而失败**

  Run: `cargo test -p textora-markdown --lib drag_preview_projects_only_the_source_subtree_rectangles`

  Expected: FAIL，错误指出 `CanvasDragPreview` 没有 `source_subtree_rects` 字段。

- [ ] **Step 3: 扩展纯数据协议并投影源子树布局矩形**

  在 `CanvasDragPreview` 的 `source_rect` 后添加：

  ```rust
  pub source_subtree_rects: Vec<Rect>,
  ```

  在 `MindmapView::calculate_drag_preview` 中，以 DFS 节点地址对应布局索引，收集源节点和所有后代的 `layout_rect`：

  ```rust
  let source_subtree_rects = collect_nodes_dfs(source_node)
      .iter()
      .filter_map(|node| nodes.iter().position(|candidate| std::ptr::eq(*candidate, *node)))
      .filter_map(|index| layout.nodes.get(index))
      .map(layout_rect)
      .collect();
  ```

  更新所有 `CanvasDragPreview` 测试构造器，使用空 `Vec` 或明确的源子树矩形。

- [ ] **Step 4: 运行协议投影测试与 UI 编译检查**

  Run: `cargo test -p textora-markdown --lib drag_preview_projects_only_the_source_subtree_rectangles`

  Expected: PASS。

  Run: `cargo check -p textora-ui`

  Expected: PASS。

- [ ] **Step 5: 提交协议投影改动**

  ```bash
  git add crates/ui/src/plugin.rs crates/markdown/src/mindmap_view.rs
  git commit -m "feat(ui): expose mmap drag source subtree geometry"
  ```

### Task 3: 以纯 UI 矩形置灰完整源子树

**Files:**

- Modify: `crates/markdown/src/mmf/canvas.rs:128-190, 430-760, 977-1140`

**Interfaces:**

- Consumes: `CanvasDragPreview::source_subtree_rects: Vec<Rect>`。
- Produces: 画布渲染中，属于源子树的卡片、标题、选中框和连线均乘以 `theme.mindmap.geometry.drag_source_alpha`。

- [ ] **Step 1: 写失败的绘制命令测试**

  在 `mmf/canvas.rs` 添加一个三节点布局（根、源、源子），构造 `source_subtree_rects` 包含源和源子。渲染后断言：源与源子卡片填充色 alpha 为原样式 alpha 乘 `drag_source_alpha`；源子连接线 alpha 同样降低；根卡片与根到源的连接线保持原 alpha；源子标题使用降低后的 alpha。

  ```rust
  #[test]
  fn drag_preview_dims_every_source_subtree_visual() {
      // 检查 FillRect、StrokeRect 与 TextShaped 命令的颜色 alpha。
  }
  ```

- [ ] **Step 2: 运行测试，确认当前渲染只覆盖源卡片背景而失败**

  Run: `cargo test -p textora-markdown --lib drag_preview_dims_every_source_subtree_visual`

  Expected: FAIL，源子卡片、文本或内部连接线仍使用完整 alpha。

- [ ] **Step 3: 将透明度传入各渲染入口**

  在 canvas 添加两个小型 helper：

  ```rust
  fn source_subtree_opacity(preview: Option<&CanvasDragPreview>, rect: Rect, theme: &Theme) -> f32;
  fn is_source_subtree_node(preview: &CanvasDragPreview, rect: Rect) -> bool;
  ```

  通过可选预览把 opacity 传给 `render_connectors`、`render_cards`、`render_text` 与 `render_node_selection`。连接线依据子节点卡片矩形判定，因此父到源的连线保持完整，源到后代的连线置灰。用 `with_alpha` 处理填充、描边、文字和选中框颜色。删除旧的 `render_drag_source` 背景覆盖函数，浮动预览仍根据 `source_rect` 查找源节点样式。

- [ ] **Step 4: 运行画布测试与 markdown 全库测试**

  Run: `cargo test -p textora-markdown --lib drag_preview_dims_every_source_subtree_visual`

  Expected: PASS。

  Run: `cargo test -p textora-markdown --lib mmf::canvas`

  Expected: PASS。

  Run: `cargo test -p textora-markdown --lib`

  Expected: PASS。

- [ ] **Step 5: 格式化、检查并提交渲染改动**

  ```bash
  cargo fmt
  cargo fmt --check
  cargo check -p textora-markdown
  git add crates/markdown/src/mmf/canvas.rs
  git commit -m "fix(markdown): dim mmap drag source subtree"
  ```

### Task 4: 全面验证

**Files:**

- Verify only: `crates/ui/src/plugin.rs`, `crates/markdown/src/mindmap_view.rs`, `crates/markdown/src/mmf/canvas.rs`

- [ ] **Step 1: 检查工作树和补丁范围**

  Run: `git status --short && git diff HEAD~2..HEAD --check`

  Expected: 仅设计、计划和 mmap 拖拽相关文件；无空白错误。

- [ ] **Step 2: 运行项目完整验证**

  Run: `./scripts/verify.sh`

  Expected: exit 0。

- [ ] **Step 3: 记录最终证据**

  Run: `git log --oneline -3 && git status --short`

  Expected: 显示本任务提交，工作树干净。
