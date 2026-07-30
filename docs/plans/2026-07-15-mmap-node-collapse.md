# 思维导图节点展开与收起 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让非根思维导图节点可通过连接线转折点上的 `+/-` 持久化展开或收起，并让拖拽预览显示标题和完整后代数量。

**Architecture:** 解析器记录节点属性块的精确源码范围，`mmf::edit` 据此生成局部 `EditTransaction`。布局只生成可见节点并以原始 DFS 索引关联 AST；画布基于该映射绘制计数、悬浮控件与带文字的拖拽预览。通用插件协议只传递控件命中、控件事务查询与画布指针位置，app 不依赖任何思维导图状态。

**Tech Stack:** Rust、MMF 解析器、`ui::plugin`、自定义画布渲染、`cargo test`。

## Global Constraints

- 根节点始终展开，不能显示或激活折叠控件。
- 折叠状态必须写回 `toml node` 的 `collapsed = true/false`。
- 折叠计数是完整后代子树总数，不受内部折叠状态影响。
- 控件仅在节点卡片或控件命中区域悬浮时绘制；控件区域不能编辑、选中或拖拽节点。
- 卡片悬浮时轻微提亮背景并使用 `connector_hover` 描边；仅悬浮控件区域时不提亮卡片。
- 拖拽预览始终是一张收起卡片，显示标题和完整后代数，不显示控件或后代。
- UI 层不得依赖 `MindmapView`、`Tree`、`Node` 或任何 app 状态。
- 每个任务先写失败测试，再写最小实现；每个任务完成后运行列出的测试和 `cargo fmt`。

---

## File Structure

| 文件 | 职责 |
| --- | --- |
| `crates/markdown/src/mmf/model.rs` | 节点属性块与 `collapsed` 值的源码元数据。 |
| `crates/markdown/src/mmf/parser.rs` | 构造源码元数据，保持 MMF AST 只读投影。 |
| `crates/markdown/src/mmf/edit.rs` | 生成折叠状态切换的局部事务。 |
| `crates/markdown/src/mmf/layout.rs` | 生成可见节点布局、原始 DFS 映射和控件命中几何。 |
| `crates/ui/src/plugin.rs` | 与具体插件无关的画布控件协议。 |
| `crates/app/src/dispatch/mouse.rs` | 分派画布悬浮和控件事务，阻止控件进入拖拽。 |
| `crates/markdown/src/mmf/canvas.rs` | 绘制折叠计数、`+/-` 和带标题的拖拽预览。 |
| `crates/markdown/src/mindmap_view.rs` | 聚合布局、命中、折叠事务、悬浮状态和拖拽预览。 |

### Task 1: 记录属性源码并生成折叠事务

**Files:**

- Modify: `crates/markdown/src/mmf/model.rs`
- Modify: `crates/markdown/src/mmf/parser.rs`
- Modify: `crates/markdown/src/mmf/edit.rs`

**Interfaces:**

- Produces `NodePropertySource { body_range: Range<usize>, collapsed_value_range: Option<Range<usize>> }`、`Node::property_source` 和 `Node::heading_source_end`。
- Produces `pub(crate) fn plan_toggle_collapsed(tree: &Tree, source: &str, node_range: Range<usize>, source_generation: u32) -> EditPlan`。

- [ ] **Step 1: 在 `parser.rs` 和 `edit.rs` 添加失败测试。**

  断言解析器分别记录已有值、缺失字段和缺失属性块；断言事务只替换 `true/false`、只向属性块正文插入字段、或在标题后插入最小块；断言根节点和叶子节点返回 `EditPlan::Consume`。

  ```rust
  #[test]
  fn toggle_collapsed_replaces_only_existing_boolean_value() {
      let source = "# Root\n## Child\n```toml node\ncollapsed = false\npriority = \"P1\"\n```\n";
      let tree = parser::parse(source).expect("fixture must parse");
      let child = &tree.root.children[0];
      let EditPlan::Apply(transaction) = plan_toggle_collapsed(
          &tree, source, child.subtree_source_range.clone(), 7,
      ) else { panic!("toggle must apply a transaction"); };
      assert_eq!(transaction.replacements, vec![TextReplacement {
          range: child.property_source.as_ref().expect("property source").collapsed_value_range
              .clone().expect("collapsed value"),
          text: "true".into(),
      }]);
  }
  ```

- [ ] **Step 2: 运行失败测试。**

  Run: `cargo test -p textora-markdown mmf::parser::tests:: -- --nocapture && cargo test -p textora-markdown mmf::edit::tests::toggle_collapsed -- --nocapture`

  Expected: 编译失败，提示 `property_source` 或 `plan_toggle_collapsed` 未定义。

- [ ] **Step 3: 增加精确范围并实现最小事务。**

  在模型中增加以下类型和字段；所有解析构造点（根、普通节点和测试夹具）都初始化新字段。

  ```rust
  #[derive(Debug, Clone)]
  pub struct NodePropertySource {
      pub body_range: Range<usize>,
      pub collapsed_value_range: Option<Range<usize>>,
  }

  pub struct Node {
      // existing fields
      pub property_source: Option<NodePropertySource>,
      pub heading_source_end: usize,
  }
  ```

  让 `collect_toml_block` 返回正文及其绝对 `body_range`；在 `parse_node_props` 之后以逐行扫描正文的方式定位键名严格等于 `collapsed` 的布尔值范围。`plan_toggle_collapsed` 通过 `subtree_source_range` 找节点，拒绝索引 `0` 和空 `children`；按下面顺序生成一条 replacement：

  ```rust
  match (&node.property_source, node.props.as_ref()) {
      (Some(source), Some(props)) if source.collapsed_value_range.is_some() => {
          TextReplacement { range: source.collapsed_value_range.clone().expect("matched above"),
              text: (!props.collapsed).to_string() }
      }
      (Some(source), Some(props)) => TextReplacement {
          range: source.body_range.end..source.body_range.end,
          text: format!("collapsed = {}\n", !props.collapsed),
      },
      (None, _) => TextReplacement {
          range: node.heading_source_end..node.heading_source_end,
          text: "\n```toml node\ncollapsed = true\n```\n".into(),
      },
  }
  ```

  `selection_after` 保持 `EditSelection::Caret(node.title_byte_range.end)`；事务使用传入的 `source_generation`。

- [ ] **Step 4: 格式化并验证。**

  Run: `cargo fmt --check && cargo test -p textora-markdown mmf::parser::tests:: && cargo test -p textora-markdown mmf::edit::tests::toggle_collapsed -- --nocapture`

  Expected: 全部 PASS。

- [ ] **Step 5: 提交。**

  ```bash
  git add crates/markdown/src/mmf/model.rs crates/markdown/src/mmf/parser.rs crates/markdown/src/mmf/edit.rs
  git commit -m "feat(mindmap): persist collapsed node state"
  ```

### Task 2: 构建可见节点布局与控件命中图

**Files:**

- Modify: `crates/markdown/src/mmf/layout.rs`

**Interfaces:**

- Produces `LayoutNode::source_node_index`，布局数组下标只表示可见节点序号。
- Produces `ControlHitGeometry { source_node_index: usize, bounds: Rect }` 和 `HitMap::controls`。
- Produces `pub fn descendant_count(node: &Node) -> usize`。

- [ ] **Step 1: 编写布局失败测试。**

  ```rust
  #[test]
  fn collapsed_node_hides_all_descendants_but_retains_original_dfs_index() {
      let tree = parser::parse("# Root\n## A\n```toml node\ncollapsed = true\n```\n### B\n## C\n")
          .expect("fixture must parse");
      let layout = compute_layout(&tree, &mut test_shaper(), &LayoutConstants::default(), None);
      assert_eq!(layout.nodes.iter().map(|node| node.source_node_index).collect::<Vec<_>>(), vec![0, 1, 3]);
      assert!(layout.nodes.iter().all(|node| node.source_node_index != 2));
  }
  ```

  同一模块增加：收起节点计数为完整后代数、根和叶子没有控件、普通分支控件中心等于其所有子连接线的共享转折点。

- [ ] **Step 2: 运行失败测试。**

  Run: `cargo test -p textora-markdown mmf::layout::tests::collapsed -- --nocapture`

  Expected: FAIL，`source_node_index`、`controls` 或可见遍历尚不存在。

- [ ] **Step 3: 实现可见 DFS 遍历。**

  用返回“下一个完整 DFS 索引”的递归替代依赖数组位置的递归。对每个节点先生成自身；若该节点不是根且 `node.props.as_ref().is_some_and(|props| props.collapsed)`，调用 `subtree_node_count` 跳过整个后代范围且不递归。`collect_card_widths_by_depth`、`assign_positions`、`build_hit_map` 都使用同一可见遍历顺序。

  ```rust
  fn is_expanded(node: &Node, source_node_index: usize) -> bool {
      source_node_index == 0 || !node.props.as_ref().is_some_and(|props| props.collapsed)
  }

  pub fn descendant_count(node: &Node) -> usize {
      node.children.iter().map(|child| 1 + descendant_count(child)).sum()
  }
  ```

  `LayoutNode`、`NodeHitGeometry` 和所有从 layout 取 AST 节点的调用改为 `source_node_index` 查找。为每个非根非叶的可见节点生成 `bounds`，其中心为 `(first_child.connector_turn_x, node_center_y)`；`HitMap::controls` 只保留可见控件。

- [ ] **Step 4: 格式化并验证。**

  Run: `cargo fmt --check && cargo test -p textora-markdown mmf::layout::tests:: -- --nocapture`

  Expected: 全部 PASS，原布局测试也保持通过。

- [ ] **Step 5: 提交。**

  ```bash
  git add crates/markdown/src/mmf/layout.rs
  git commit -m "feat(mindmap): lay out only expanded nodes"
  ```

### Task 3: 增加通用画布控件协议与 app 分派

**Files:**

- Modify: `crates/ui/src/plugin.rs`
- Modify: `crates/app/src/dispatch/mouse.rs`

**Interfaces:**

- Produces `EditHitTarget::CanvasControl { source_range: Range<usize> }`。
- Produces `PluginQuery::PlanCanvasControl { source_range: Range<usize>, source_generation: u32 }` 与 `PluginResponse::EditPlan(EditPlan)`。
- Produces `PluginMessage::SetCanvasPointer(Option<CanvasPoint>)`。

- [ ] **Step 1: 在 `mouse.rs` 测试模块写失败的协议测试。**

  测试插件对控件命中返回 `CanvasControl`，对 `PlanCanvasControl` 返回替换事务。断言按下后 app 执行事务并同步插件；断言未创建 `CanvasDragSession`。另一个测试调用 `dispatch_editor_cursor_moved`，断言测试插件收到最新 `SetCanvasPointer(Some(...))`。

  ```rust
  assert_eq!(state.borrow().planned_control_ranges, vec![1..4]);
  assert!(state.borrow().drag_requests.is_empty());
  assert_eq!(document_text(&app), "aTRUEc");
  ```

- [ ] **Step 2: 运行失败测试。**

  Run: `cargo test -p textora-app --lib -- canvas_control --nocapture`

  Expected: 编译失败，新的命中目标、查询或响应尚不存在。

- [ ] **Step 3: 定义协议并在 app 执行事务。**

  在 `plugin.rs` 增加上述枚举变体。`PluginResponse::EditPlan` 复用已定义的 `EditPlan`，不引入 mindmap 类型。

  在 custom-renderer 按下分支中、`SourceObject` 分支之前处理控件：调用 `PlanCanvasControl`，仅在返回 `EditPlan::Apply(transaction)` 时调用 `execute_edit_plan`，随后调用 `sync_plugin_state()`，设置 `mouse.is_down = false` 并返回 `REDRAW`。其余 plan 结果不修改文档。每次 custom-renderer 指针移动都先发送 `SetCanvasPointer(Some(CanvasPoint::new(px, py)))`，再处理已有拖拽逻辑。

- [ ] **Step 4: 格式化并验证。**

  Run: `cargo fmt --check && cargo test -p textora-app --lib -- canvas_control --nocapture && cargo test -p textora-app --lib -- mouse --nocapture`

  Expected: PASS，既有 canvas 拖拽生命周期测试不变。

- [ ] **Step 5: 提交。**

  ```bash
  git add crates/ui/src/plugin.rs crates/app/src/dispatch/mouse.rs
  git commit -m "feat(canvas): dispatch control edit plans"
  ```

### Task 4: 接入思维导图控件、折叠标签与可见导航

**Files:**

- Modify: `crates/markdown/src/mindmap_view.rs`
- Modify: `crates/markdown/src/mmf/canvas.rs`

**Interfaces:**

- Consumes Task 1 的 `plan_toggle_collapsed`、Task 2 的 `HitMap::controls`、Task 3 的画布控件协议。
- Produces `MindmapRenderProjection::collapsed_descendant_counts` 和 `CanvasDragPreview::label`。

- [ ] **Step 1: 添加 mindmap/canvas 失败测试。**

  覆盖以下独立断言：

  ```rust
  // controls precede title/card hit testing
  assert!(matches!(view.semantic_hit_target(control.x, control.y),
      Some(EditHitTarget::CanvasControl { .. })));
  // root never has a control
  assert!(view.ready_hit_map().controls.iter().all(|control| control.source_node_index != 0));
  // collapsed card and preview expose the entire descendant count
  assert_eq!(projection.collapsed_descendant_counts[child_index], Some(3));
  assert_eq!(preview.canvas.label, "Child · 3");
  ```

  画布 draw-list 测试断言：悬浮卡片时存在 `connector_hover` 描边和比原填充色更高 alpha 的填充命令；仅悬浮控件时不存在该提亮卡片命令；悬浮时存在 `+` 或 `-` 的 text 命令、非悬浮时不存在；拖拽预览绘制 `label`，且没有控件文字。

- [ ] **Step 2: 运行失败测试。**

  Run: `cargo test -p textora-markdown mindmap_view::tests::collapse -- --nocapture && cargo test -p textora-markdown mmf::canvas::tests::drag_preview -- --nocapture`

  Expected: FAIL，控件命中、投影计数或预览标签尚不存在。

- [ ] **Step 3: 实现命中、悬浮与标签渲染。**

  `MindmapView` 保存最后一处 `canvas_pointer`。处理 `SetCanvasPointer` 时仅更新该字段；处理 `PlanCanvasControl` 时调用：

  ```rust
  mmf::edit::plan_toggle_collapsed(tree, source, source_range, source_generation)
  ```

  `semantic_hit_target` 必须先检查 `hit_map.controls`，再检查标题和卡片。控件可见性为“指针命中任意卡片或同一控件”；根与叶子永远不绘制。指针命中卡片时，`render_cards` 使用 `connector_hover` 描边并将原 fill 的 alpha 乘以语义化的悬浮提亮系数；指针仅命中控件时不改变卡片样式。`visible_dfs_neighbor` 改为基于 `layout.nodes[*].source_node_index` 的相邻可见节点，避免光标导航进入隐藏后代。

  `build_render_projection` 为收起节点记录 `Some(descendant_count(node))`，但标题编辑的 grapheme hit-map 继续只映射真实标题。`canvas.rs` 在标题右侧单独绘制 ` · N`；测量折叠卡片宽度时包含同一后缀。向 `CanvasDragPreview` 增加 `label: String`；`calculate_drag_preview` 用 `format!("{} · {}", source_node.title, descendant_count(source_node))` 赋值，并使用公开的 `measured_card_width` 计算足以容纳标签的预览宽度。`render_drag_preview` 接收 `shaper` 并在卡片内绘制 `preview.label`。

- [ ] **Step 4: 运行模块验证。**

  Run: `cargo fmt --check && cargo test -p textora-markdown mindmap_view::tests:: -- --nocapture && cargo test -p textora-markdown mmf::canvas::tests:: -- --nocapture`

  Expected: PASS，包含现有拖拽目标、源子树淡化和 IME 测试。

- [ ] **Step 5: 提交。**

  ```bash
  git add crates/markdown/src/mindmap_view.rs crates/markdown/src/mmf/canvas.rs
  git commit -m "feat(mindmap): render collapse controls and drag labels"
  ```

### Task 5: 全链路回归与验收

**Files:**

- Modify: `crates/markdown/src/mindmap_view.rs`（只在前序模块测试不足时补充端到端夹具）
- Modify: `crates/app/src/dispatch/mouse.rs`（只在前序协议测试不足时补充端到端夹具）

**Interfaces:**

- Consumes Tasks 1–4 的全部公开接口。
- Produces 最终回归证据，不新增生产接口。

- [ ] **Step 1: 增加端到端失败用例。**

  用 `# Root / ## Parent / ### Child / #### Grandchild` 夹具模拟控件命中后执行计划、同步源码并再次渲染。断言源码包含 `collapsed = true`、可见布局只含 Root 与 Parent、Parent 文本带 `· 2`、拖拽 Parent 的预览标签为 `Parent · 2`、根控件点击无事务。

- [ ] **Step 2: 运行用例并确认失败原因是缺失串联。**

  Run: `cargo test -p textora-markdown mindmap_view::tests::collapse_end_to_end -- --nocapture && cargo test -p textora-app --lib -- canvas_control_end_to_end --nocapture`

  Expected: 在 Task 4 前失败；Task 4 完成后 PASS。

- [ ] **Step 3: 仅修复测试揭示的接口串联缺口。**

  不改变已确认的交互：不得为根添加控件，不得自动展开落点，不得让隐藏节点成为拖拽候选。修复后重新运行同一用例。

- [ ] **Step 4: 执行完整验证。**

  Run: `cargo fmt --check && ./scripts/verify.sh`

  Expected: 两条命令均以退出码 0 结束。

- [ ] **Step 5: 提交。**

  ```bash
  git add crates/markdown/src/mindmap_view.rs crates/app/src/dispatch/mouse.rs
  git commit -m "test(mindmap): cover collapse interaction flow"
  ```

## Self-Review

- 规范覆盖：Task 1 覆盖 MMF 持久化；Task 2 覆盖隐藏布局、计数和控件几何；Task 3 覆盖跨层协议；Task 4 覆盖悬浮、`+/-`、拖拽文字和可见落点；Task 5 覆盖端到端验收和项目级验证。
- 边界一致性：根节点在 Task 1、2、4、5 均被拒绝或排除；后代计数在 Task 2 和 Task 4 均定义为完整子树；拖拽预览不使用展开状态。
- 类型一致性：Task 1 的 `plan_toggle_collapsed` 由 Task 4 调用；Task 2 的 `source_node_index` 和 `controls` 由 Task 4 消费；Task 3 的 `CanvasControl`、`PlanCanvasControl`、`EditPlan` 在 Task 4 和 app 侧同名使用。
