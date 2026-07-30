# mmap 视图主题样式完善（字号分级 + 分支配色）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** mmap 视图按层级拉开字号与卡片尺寸差距，并按"根的一级子树"为单位给分支染上不同颜色（连接线 + 卡片边框/accent）。

**Architecture:** 主题层（`ui::theme`）新增 `depth_font_scales`（几何属性，下标即深度，0=根）与 `branch_palette`（分支调色板）；布局层（`mmf::layout`）给 `LayoutNode` 增加 `branch_index` 并把卡片高度/测宽改为按深度缩放；渲染层（`mmf::canvas`）在 `get_node_style` 的默认外观之后、语义覆盖之前做分支染色，文本按深度字号渲染。

**Tech Stack:** Rust workspace（`crates/ui`、`crates/markdown`），wgpu 自绘 UI。

## Global Constraints

- 全程中文回复；代码注释/提交信息遵循仓库现有风格。
- 语义覆盖优先级不变：分支染色 < status < priority(accent/border) < named color。
- 卡片高度不新增独立配置，一律 `card_height × font_scale(depth)` 推导。
- 严禁 `.unwrap()`（除测试与既有 `parse_hex(...).unwrap()` 模式外）；遵守 `cargo fmt`。
- 每个 Task 结束必须 `cargo test -p <相关包>` 通过并编译无警告。
- 提交步骤需用户确认后执行（本会话规则）。

---

### Task 1: 主题数据结构 —— depth_font_scales 与 branch_palette

**Files:**
- Modify: `crates/ui/src/theme/mindmap.rs`
- Test: `crates/ui/src/theme/mindmap.rs`（文件内 `mod tests`）

**Interfaces:**
- Produces:
  - `MindmapGeometry::depth_font_scales: Vec<f32>`，下标即深度（0=根，1=一级……），默认 `vec![1.35, 1.15, 1.0, 0.9]`
  - `MindmapGeometry::font_scale_for_depth(&self, depth: u8) -> f32`（越界取最后一项，空数组回退 1.0）
  - `MindmapCanvasTheme::branch_palette: Vec<[f32; 4]>`
  - `MindmapCanvasTheme::branch_color(&self, branch_index: usize) -> Option<[f32; 4]>`（取模循环，空数组返回 None）

- [ ] **Step 1: 写失败测试**

在 `crates/ui/src/theme/mindmap.rs` 的 `mod tests` 中追加：

```rust
    #[test]
    fn font_scale_for_depth_clamps_to_last_entry() {
        let geometry = MindmapGeometry::default();

        assert_eq!(geometry.font_scale_for_depth(0), 1.35);
        assert_eq!(geometry.font_scale_for_depth(1), 1.15);
        assert_eq!(geometry.font_scale_for_depth(2), 1.0);
        assert_eq!(geometry.font_scale_for_depth(3), 0.9);
        assert_eq!(geometry.font_scale_for_depth(9), 0.9);
    }

    #[test]
    fn font_scale_for_depth_falls_back_to_one_when_empty() {
        let geometry = MindmapGeometry { depth_font_scales: vec![], ..Default::default() };

        assert_eq!(geometry.font_scale_for_depth(0), 1.0);
    }

    #[test]
    fn branch_color_cycles_through_palette() {
        let dark = MindmapTheme::default_dark();
        let palette_len = dark.canvas.branch_palette.len();
        assert!(palette_len >= 6, "dark palette should distinguish many branches");

        let first = dark.canvas.branch_color(0).expect("palette is non-empty");
        let cycled = dark.canvas.branch_color(palette_len).expect("palette is non-empty");
        assert_eq!(first, cycled);
        assert_ne!(first, dark.canvas.branch_color(1).expect("palette is non-empty"));

        let light = MindmapTheme::default_light();
        assert!(light.canvas.branch_palette.len() >= 6);
    }

    #[test]
    fn branch_color_returns_none_for_empty_palette() {
        let mut dark = MindmapTheme::default_dark();
        dark.canvas.branch_palette.clear();

        assert_eq!(dark.canvas.branch_color(0), None);
    }

    #[test]
    fn gamma_correct_also_corrects_branch_palette() {
        let mut theme = MindmapTheme::default_dark();
        let original = theme.canvas.branch_palette[0];

        theme.gamma_correct();

        let corrected = theme.canvas.branch_palette[0];
        assert!((corrected[0] - original[0].powf(2.2)).abs() < 1e-6, "RGB must be gamma-corrected");
        assert!((corrected[3] - original[3]).abs() < f32::EPSILON, "alpha must not be gamma-corrected");
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-ui --lib theme::mindmap 2>&1 | tail -20`
Expected: 编译失败，`depth_font_scales` / `branch_palette` 字段不存在。

- [ ] **Step 3: 实现**

`crates/ui/src/theme/mindmap.rs` 中：

`MindmapCanvasTheme` 增加字段：

```rust
#[derive(Debug, Clone)]
pub struct MindmapCanvasTheme {
    pub background: [f32; 4],
    pub connector: [f32; 4],
    pub connector_hover: [f32; 4],
    pub selection: [f32; 4],
    pub focus_ring: [f32; 4],
    pub drag_invalid: [f32; 4],
    /// 根的一级子树分支调色板，按 branch_index 取模循环。
    pub branch_palette: Vec<[f32; 4]>,
}

impl MindmapCanvasTheme {
    /// 返回分支染色；调色板为空时返回 None，调用方回退到默认色。
    pub fn branch_color(&self, branch_index: usize) -> Option<[f32; 4]> {
        if self.branch_palette.is_empty() {
            return None;
        }
        Some(self.branch_palette[branch_index % self.branch_palette.len()])
    }
}
```

`MindmapGeometry` 增加字段与 helper：

```rust
#[derive(Debug, Clone)]
pub struct MindmapGeometry {
    // ……既有字段保持不动……
    pub same_level_threshold_ratio: f32,
    /// 各深度字号缩放，下标即深度（0=根）。卡片高度同比例推导。
    pub depth_font_scales: Vec<f32>,
}

impl MindmapGeometry {
    /// 深度越界时钳制到最后一档，空数组回退 1.0。
    pub fn font_scale_for_depth(&self, depth: u8) -> f32 {
        if self.depth_font_scales.is_empty() {
            return 1.0;
        }
        let index = (depth as usize).min(self.depth_font_scales.len() - 1);
        self.depth_font_scales[index]
    }
}
```

`MindmapGeometry::default()` 末尾追加 `depth_font_scales: vec![1.35, 1.15, 1.0, 0.9],`。

`gamma_correct()` 追加：

```rust
        for c in &mut self.canvas.branch_palette {
            correct(c);
        }
```

`default_dark()` 的 `canvas` 初始化追加（沿用既有色族，新增紫/青/粉补满 6 色）：

```rust
                branch_palette: vec![
                    h("#5DA9E9"),
                    h("#62C370"),
                    h("#F2A65A"),
                    h("#B57EDC"),
                    h("#4EC9B0"),
                    h("#E06C9F"),
                ],
```

`default_light()` 的 `canvas` 初始化追加（与浅色既有色族同饱和度的深色系）：

```rust
                branch_palette: vec![
                    h("#4F8FCF"),
                    h("#2F9E58"),
                    h("#D9822B"),
                    h("#8E6CC8"),
                    h("#2A9D8F"),
                    h("#C2537E"),
                ],
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p textora-ui --lib theme::mindmap`
Expected: 全部 PASS（含既有测试）。

- [ ] **Step 5: 提交（需用户确认）**

```bash
git add crates/ui/src/theme/mindmap.rs
git commit -m "feat(theme): mindmap 增加 depth_font_scales 与 branch_palette"
```

---

### Task 2: 主题文件格式支持

**Files:**
- Modify: `crates/ui/src/theme_file.rs`（`MindmapCanvasFile` 约 :125、`MindmapGeometryFile` :181、`resolve_mindmap` :379）
- Test: `crates/ui/src/theme_file.rs`（文件内测试模块；若无则在末尾新建 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: Task 1 的 `MindmapCanvasTheme::branch_palette`、`MindmapGeometry::depth_font_scales`。
- Produces: 主题文件新键 `mindmap.canvas.branch_palette = ["#RRGGBB", ...]`、`mindmap.geometry.depth_font_scales = [1.35, ...]`。

- [ ] **Step 1: 写失败测试**

先查看 `theme_file.rs` 末尾是否已有 `mod tests`（有则沿用其构造 ThemeFile 的既有模式），追加：

```rust
    #[test]
    fn resolves_branch_palette_and_depth_font_scales() {
        let toml_src = r##"
            [mindmap.canvas]
            branch_palette = ["#FF0000", "#00FF00"]

            [mindmap.geometry]
            depth_font_scales = [1.5, 1.2]
        "##;
        let file: ThemeFile = toml::from_str(toml_src).expect("valid toml");
        let mut theme = Theme::default_dark();
        file.resolve_into(&mut theme).expect("resolve should succeed");

        assert_eq!(theme.mindmap.canvas.branch_palette.len(), 2);
        assert_eq!(theme.mindmap.geometry.depth_font_scales, vec![1.5, 1.2]);
        assert_eq!(theme.mindmap.geometry.font_scale_for_depth(9), 1.2);
    }
```

注意：测试里 `ThemeFile` 的解析入口与 `resolve_into` 的确切名字以 `theme_file.rs` 现有公开 API 为准（写测试前先读该文件 `impl ThemeFile` 与既有测试的调用方式，照搬之）。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-ui --lib theme_file 2>&1 | tail -20`
Expected: 编译失败，`branch_palette` / `depth_font_scales` 未知字段（`deny_unknown_fields` 会直接反序列化报错，亦为可接受的失败形态）。

- [ ] **Step 3: 实现**

`MindmapCanvasFile` 增加：

```rust
    pub branch_palette: Option<Vec<String>>,
```

`MindmapGeometryFile` 增加：

```rust
    pub depth_font_scales: Option<Vec<f32>>,
```

`resolve_mindmap()` 的 canvas 分支末尾追加：

```rust
        if let Some(ref palette) = c.branch_palette {
            let mut resolved = Vec::with_capacity(palette.len());
            for (i, hex) in palette.iter().enumerate() {
                resolved.push(parse_hex_field(hex, &format!("mindmap.canvas.branch_palette[{}]", i))?);
            }
            target.canvas.branch_palette = resolved;
        }
```

geometry 分支追加（先看 `resolve_mindmap` 里 geometry 是如何逐项 `apply` 的，沿用同款模式）：

```rust
        if let Some(scales) = g.depth_font_scales {
            target.geometry.depth_font_scales = scales;
        }
```

`parse_hex_field` 若不存在，则用文件内既有的 hex 解析 + `ResolveError::InvalidHex` 路径（照搬 `apply_color` 的实现方式）。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p textora-ui --lib theme_file`
Expected: PASS。

- [ ] **Step 5: 提交（需用户确认）**

```bash
git add crates/ui/src/theme_file.rs
git commit -m "feat(theme): 主题文件支持 branch_palette 与 depth_font_scales"
```

---

### Task 3: LayoutConstants 携带字号缩放

**Files:**
- Modify: `crates/markdown/src/mmf/layout.rs:23-61`（`LayoutConstants`）
- Modify: `crates/markdown/src/mindmap_view.rs:148-160`（`apply_theme` 构造 `next_constants` 处）
- Test: `crates/markdown/src/mmf/layout.rs`（文件内 `mod tests`）

**Interfaces:**
- Produces:
  - `LayoutConstants::depth_font_scales: Vec<f32>`
  - `LayoutConstants::font_scale_for_depth(&self, depth: u8) -> f32`
  - `LayoutConstants::card_height_for_depth(&self, depth: u8) -> f32`（= `card_height × font_scale_for_depth(depth)`）
- 后续 Task 4-8 全部经由这两个 helper 取缩放，禁止各自重算。

- [ ] **Step 1: 写失败测试**

`layout.rs` 的 `mod tests` 追加：

```rust
    #[test]
    fn card_height_scales_with_depth_font_scale() {
        let constants = LayoutConstants::default();

        let root_h = constants.card_height_for_depth(0);
        let level2_h = constants.card_height_for_depth(2);
        assert!(root_h > level2_h, "root card must be taller than level-2 card");
        assert_eq!(level2_h, constants.card_height);
        assert_eq!(constants.card_height_for_depth(9), constants.card_height_for_depth(3));
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-markdown --lib mmf::layout 2>&1 | tail -20`
Expected: 编译失败，方法不存在。（包名以 `crates/markdown/Cargo.toml` 的 `name` 为准，AGENTS.md 记载为 `textora-markdown`。）

- [ ] **Step 3: 实现**

`layout.rs` 的 `LayoutConstants` 增加字段与 helper（默认缩放取自 `ui::theme::MindmapGeometry::default()`，保持单一数据源）：

```rust
#[derive(Clone, PartialEq, Debug)]
pub struct LayoutConstants {
    // ……既有字段保持不动……
    pub expanded_control_right_offset: f32,
    /// 各深度字号缩放，下标即深度（0=根）；来自 theme.mindmap.geometry。
    pub depth_font_scales: Vec<f32>,
}

impl LayoutConstants {
    pub fn scaled(dpi_scale: f32) -> Self {
        Self {
            // ……既有字段保持不动……
            expanded_control_right_offset: EXPANDED_CONTROL_RIGHT_OFFSET_DP * dpi_scale,
            depth_font_scales: ui::theme::MindmapGeometry::default().depth_font_scales,
        }
    }

    /// 深度越界钳制到最后一档，空数组回退 1.0。
    pub fn font_scale_for_depth(&self, depth: u8) -> f32 {
        if self.depth_font_scales.is_empty() {
            return 1.0;
        }
        let index = (depth as usize).min(self.depth_font_scales.len() - 1);
        self.depth_font_scales[index]
    }

    /// 卡片高度随字号缩放推导，不单独配置。
    pub fn card_height_for_depth(&self, depth: u8) -> f32 {
        self.card_height * self.font_scale_for_depth(depth)
    }
}
```

`mindmap_view.rs:150` 的 `next_constants` 构造追加：

```rust
            depth_font_scales: geometry.depth_font_scales.clone(),
```

（该行需插入在结构体字面量内；同时确认 `next_constants` 与 `self.constants` 的 `PartialEq` 比较因此自动覆盖主题切换。）

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p textora-markdown --lib mmf::layout`
Expected: PASS。

- [ ] **Step 5: 提交（需用户确认）**

```bash
git add crates/markdown/src/mmf/layout.rs crates/markdown/src/mindmap_view.rs
git commit -m "feat(mmf): LayoutConstants 携带 depth_font_scales 并提供按深度高度"
```

---

### Task 4: LayoutNode 增加 branch_index

**Files:**
- Modify: `crates/markdown/src/mmf/layout.rs`（`LayoutNode` :63-78、`assign_positions` :164-228、`compute_layout` :297-337）
- Test: `crates/markdown/src/mmf/layout.rs`（文件内 `mod tests`）

**Interfaces:**
- Produces: `LayoutNode::branch_index: Option<usize>` —— 根为 `None`；根的第 N 个（可见）孩子为 `Some(N)`；更深层节点继承其一级祖先的值。Task 7 的消费方：`canvas.rs` 的 `get_node_style` 与 `render_connectors`。

- [ ] **Step 1: 写失败测试**

先看 `layout.rs` 既有测试如何构造 `Tree`/`Node`（文件底部测试有现成 helper，照搬），追加：

```rust
    #[test]
    fn branch_index_tracks_top_level_ancestor() {
        // 构造: root ─┬─ A ── A1
        //            └─ B
        let tree = build_test_tree(); // 沿用文件内既有构造 helper，无则手写 Node 字面量
        let mut shaper = Shaper::new().expect("shaper init");
        let layout = compute_layout(&tree, &mut shaper, &LayoutConstants::default(), None);

        let by_title = |title: &str| {
            // 既有测试若已有按 source_node_index 查 LayoutNode 的模式则沿用
            layout.nodes.iter().find(|n| /* 标题匹配 */ false).cloned()
        };
        // 断言：root.branch_index == None
        assert_eq!(layout.nodes[0].branch_index, None);
        // A、A1 的 branch_index == Some(0)；B == Some(1)
        // （按 DFS 序：root=0, A=1, A1=2, B=3）
        assert_eq!(layout.nodes[1].branch_index, Some(0));
        assert_eq!(layout.nodes[2].branch_index, Some(0));
        assert_eq!(layout.nodes[3].branch_index, Some(1));
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-markdown --lib mmf::layout 2>&1 | tail -20`
Expected: 编译失败，`branch_index` 字段不存在。

- [ ] **Step 3: 实现**

`LayoutNode` 增加：

```rust
    /// 所属一级分支序号（根为 None，根的第 N 个孩子为 Some(N)，更深层继承）。
    pub branch_index: Option<usize>,
```

`assign_positions` 签名加参数 `branch_index: Option<usize>`，函数体：

```rust
    out.push(LayoutNode {
        // ……既有字段……
        connector_turn_x: parent_connector_turn_x,
        branch_index,
    });
```

子节点循环中计算孩子的 branch_index（根的孩子们各自领序号，其余直接继承）：

```rust
    for (child_ordinal, child) in node.children.iter().enumerate() {
        let child_h = subtree_height(child, child_source_index, constants);
        let child_x = x + card_w + constants.child_gap_for_parent_depth(depth);
        let child_connector_turn_x = (this_connector.0 + child_x) * 0.5;
        let child_branch_index = if depth == 0 { Some(child_ordinal) } else { branch_index };
        child_source_index = assign_positions(
            child,
            child_source_index,
            depth + 1,
            cursor,
            child_x,
            Some(this_connector),
            Some(child_connector_turn_x),
            child_branch_index,
            constants,
            card_widths_by_depth,
            out,
        );
        cursor += child_h + constants.sibling_gap;
    }
```

`compute_layout` 调用处传 `None`（根的 branch_index）。注意折叠节点不展开子节点，序号为"可见一级孩子的序号"，符合染色语义。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p textora-markdown --lib mmf::layout`
Expected: PASS。

- [ ] **Step 5: 提交（需用户确认）**

```bash
git add crates/markdown/src/mmf/layout.rs
git commit -m "feat(mmf): LayoutNode 记录 branch_index 以支持分支染色"
```

---

### Task 5: 布局按深度缩放卡片高度与测宽

**Files:**
- Modify: `crates/markdown/src/mmf/layout.rs`（`subtree_height` :149-162、`assign_positions` :164-228、`collect_card_widths_by_depth` :230-269、`compute_layout` :326）
- Test: `crates/markdown/src/mmf/layout.rs`（文件内 `mod tests`）

**Interfaces:**
- Consumes: Task 3 的 `card_height_for_depth(depth)`、`font_scale_for_depth(depth)`。
- Produces: 布局产出的 `LayoutNode.h` 与连线端点 y 均为按深度缩放后的值；`LayoutTree::total_h` 同步正确。

- [ ] **Step 1: 写失败测试**

```rust
    #[test]
    fn layout_assigns_taller_cards_to_shallower_depths() {
        // root ── child ── grandchild
        let tree = build_chain_tree(); // 沿用/仿照既有测试构造
        let mut shaper = Shaper::new().expect("shaper init");
        let constants = LayoutConstants::default();
        let layout = compute_layout(&tree, &mut shaper, &constants, None);

        assert_eq!(layout.nodes[0].h, constants.card_height_for_depth(0));
        assert_eq!(layout.nodes[1].h, constants.card_height_for_depth(1));
        assert_eq!(layout.nodes[2].h, constants.card_height_for_depth(2));
        assert!(layout.nodes[0].h > layout.nodes[2].h);
        // 连线端点应在本卡片左边缘中点
        let node = &layout.nodes[1];
        assert_eq!(node.connector_to.1, node.y + node.h / 2.0);
    }

    #[test]
    fn wider_font_measures_wider_card_for_shallower_depth() {
        // 同一标题，根(depth 0)测得的卡宽应大于二级(depth 2)
        let mut shaper = Shaper::new().expect("shaper init");
        let constants = LayoutConstants::default();
        let title = "主题";

        let root_w = measured_card_width_for_depth(title, &constants, &mut shaper, 0);
        let level2_w = measured_card_width_for_depth(title, &constants, &mut shaper, 2);
        assert!(root_w > level2_w);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-markdown --lib mmf::layout 2>&1 | tail -20`
Expected: 编译失败/断言失败（`measured_card_width_for_depth` 不存在；高度断言失败）。

- [ ] **Step 3: 实现**

`subtree_height` 增加 `depth: u8` 参数，三处 `constants.card_height` 改为 `constants.card_height_for_depth(depth)`，递归调用传 `depth + 1`：

```rust
fn subtree_height(node: &Node, source_node_index: usize, depth: u8, constants: &LayoutConstants) -> f32 {
    if node.children.is_empty() || !is_expanded(node, source_node_index) {
        return constants.card_height_for_depth(depth);
    }
    let mut child_source_index = source_node_index + 1;
    let mut children_h = 0.0;
    for child in &node.children {
        children_h += subtree_height(child, child_source_index, depth + 1, constants);
        child_source_index += subtree_node_count(child);
    }
    children_h += (node.children.len() - 1) as f32 * constants.sibling_gap;
    children_h.max(constants.card_height_for_depth(depth))
}
```

`assign_positions` 内（card_h 提取为局部变量，就近声明）：

```rust
    let card_w = card_width_for_depth(card_widths_by_depth, depth);
    let card_h = constants.card_height_for_depth(depth);
    let sub_h = subtree_height(node, source_node_index, depth, constants);
    let card_y = y_offset + (sub_h - card_h) / 2.0;
    let connector_to = (x, card_y + card_h / 2.0);
    // LayoutNode { h: card_h, …… }
    // this_connector 用 (x + card_w, card_y + card_h / 2.0)
    // 子节点 subtree_height 调用传 depth + 1
```

`compute_layout` 的 `total_h` 改为 `subtree_height(&tree.root, 0, 0, constants)`。

新增按深度测宽（`measured_card_width` 保留原签名供 `mindmap_view.rs:882` 使用，内部抽公共逻辑）：

```rust
/// 按深度缩放字号后测量卡宽；测量期间临时调整 shaper 字号并恢复。
pub(crate) fn measured_card_width_for_depth(
    title: &str,
    constants: &LayoutConstants,
    shaper: &mut Shaper,
    depth: u8,
) -> f32 {
    let base_size = shaper.font_size();
    let scale = constants.font_scale_for_depth(depth);
    shaper.set_font_size(base_size * scale);
    let width = measured_card_width(title, constants, shaper);
    shaper.set_font_size(base_size);
    width
}
```

`collect_card_widths_by_depth` 中 `measured_card_width(title, constants, shaper)` 改为 `measured_card_width_for_depth(title, constants, shaper, depth)`。

注意 `assign_positions` 内所有对 `subtree_height` 的调用同步补 depth 实参（编译器会逐个指出）。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p textora-markdown --lib mmf::layout`
Expected: PASS（含既有测试；若有测试硬编码了旧高度需按新语义更新，属于预期内变更）。

- [ ] **Step 5: 提交（需用户确认）**

```bash
git add crates/markdown/src/mmf/layout.rs
git commit -m "feat(mmf): 卡片高度与测宽按深度字号缩放"
```

---

### Task 6: 命中几何按深度度量 grapheme 边缘

**Files:**
- Modify: `crates/markdown/src/mmf/layout.rs`（`build_hit_map` :377-432）
- Test: `crates/markdown/src/mmf/layout.rs`（文件内 `mod tests`）

**Interfaces:**
- Consumes: Task 3 的 `font_scale_for_depth(depth)`。
- Produces: `NodeHitGeometry::grapheme_edges` 与渲染文本（Task 8）使用同一缩放字号，保证点击/光标命中不漂移。

- [ ] **Step 1: 写失败测试**

```rust
    #[test]
    fn hit_map_grapheme_edges_match_depth_scaled_text_width() {
        // root(标题非空) ── child
        let tree = build_chain_tree();
        let mut shaper = Shaper::new().expect("shaper init");
        let constants = LayoutConstants::default();
        let layout = compute_layout(&tree, &mut shaper, &constants, None);
        let hit_map = build_hit_map(&tree, &layout, &mut shaper, &constants, None);

        let root_hit = &hit_map.nodes[0];
        let edges = &root_hit.grapheme_edges;
        let measured_span = edges.last().unwrap() - edges.first().unwrap();
        // 与 Task 5 的测宽路径独立复算：标题文本宽 ≈ measured_span
        let mut verify_shaper = Shaper::new().expect("shaper init");
        let expected = measured_card_width_for_depth(
            "根标题", &constants, &mut verify_shaper, 0,
        ) - 2.0 * constants.card_padding_x;
        assert!((measured_span - expected).abs() < 1.0, "grapheme edges must use depth-scaled font");
    }
```

（标题字符串以测试构造的树为准。）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-markdown --lib mmf::layout 2>&1 | tail -20`
Expected: 断言失败（edges 仍按基础字号）。

- [ ] **Step 3: 实现**

`build_hit_map` 循环体内，为每个节点按深度临时调整 shaper 字号：

```rust
    for ln in &layout.nodes {
        let Some(node) = nodes.get(ln.source_node_index) else {
            continue;
        };
        let base_size = shaper.font_size();
        shaper.set_font_size(base_size * constants.font_scale_for_depth(ln.depth));

        // ……原有 text_x / grapheme_byte_offsets / grapheme_edges 计算保持不动……

        shaper.set_font_size(base_size);

        // ……其余字段保持不动……
    }
```

`title_rect` 的高度表达式 `ln.h - 2.0 * constants.card_padding_y` 中 `ln.h` 已是按深度缩放值，无需改动。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p textora-markdown --lib mmf::layout`
Expected: PASS。

- [ ] **Step 5: 提交（需用户确认）**

```bash
git add crates/markdown/src/mmf/layout.rs
git commit -m "fix(mmf): 命中几何 grapheme 边缘按深度字号度量"
```

---

### Task 7: 分支染色 —— 节点样式与连接线

**Files:**
- Modify: `crates/markdown/src/mmf/canvas.rs`（`get_node_style` :86-142、`render_connectors` :485-513）
- Test: `crates/markdown/src/mmf/canvas.rs`（文件内 `mod tests`）

**Interfaces:**
- Consumes: Task 1 的 `MindmapCanvasTheme::branch_color`，Task 4 的 `LayoutNode::branch_index`。
- Produces: `get_node_style(node, layout_node, theme)` 对非根节点返回分支染色的 border/accent；`render_connectors` 用分支色画连接线。语义覆盖（status/priority/named）优先级不变。

- [ ] **Step 1: 写失败测试**

先看 `canvas.rs` 既有测试如何构造 `Node` / `LayoutNode` / `Theme`（文件内 :1240 附近有构造模式，照搬），追加：

```rust
    #[test]
    fn branch_color_tints_border_and_accent_for_default_style() {
        let theme = Theme::default_dark();
        let node = Node::default_for_test(); // 沿用既有测试构造
        let mut layout_node = layout_node_for_test(1); // depth=1
        layout_node.branch_index = Some(1);

        let style = get_node_style(&node, &layout_node, &theme);
        let expected = theme.mindmap.canvas.branch_color(1).expect("palette non-empty");
        assert_eq!(style.border, expected);
        assert_eq!(style.accent, expected);
        // fill/text 保持 depth 样式，不被染色
        assert_eq!(style.fill, theme.mindmap.node.depth[0].fill);
    }

    #[test]
    fn root_is_not_branch_tinted() {
        let theme = Theme::default_dark();
        let node = Node::default_for_test();
        let mut layout_node = layout_node_for_test(0);
        layout_node.branch_index = None;

        let style = get_node_style(&node, &layout_node, &theme);
        assert_eq!(style.border, theme.mindmap.node.root.border);
    }

    #[test]
    fn named_color_overrides_branch_tint() {
        let mut theme = Theme::default_dark();
        theme.mindmap.semantic.named.insert("sky".into(), ui::theme::MindmapNodeStyle {
            fill: [0.0, 0.0, 0.0, 1.0],
            border: [0.1, 0.2, 0.3, 1.0],
            text: [0.9, 0.9, 0.9, 1.0],
            accent: [0.1, 0.2, 0.3, 1.0],
        });
        let mut node = Node::default_for_test();
        node.props = Some(/* props with color = Some("sky".into())，照搬既有 props 构造 */);
        let mut layout_node = layout_node_for_test(1);
        layout_node.branch_index = Some(0);

        let style = get_node_style(&node, &layout_node, &theme);
        assert_eq!(style.border, [0.1, 0.2, 0.3, 1.0]);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-markdown --lib mmf::canvas 2>&1 | tail -20`
Expected: 断言失败（border 仍是 depth 样式色）。

- [ ] **Step 3: 实现**

`get_node_style` 在第 1 步（root/depth 样式）之后、`if let Some(props)` 之前插入：

```rust
    // 1.5 分支染色：只作用于默认外观的 border/accent，语义覆盖仍可覆盖之
    if layout_node.depth > 0 {
        if let Some(branch_color) =
            layout_node.branch_index.and_then(|index| mt.canvas.branch_color(index))
        {
            style.border = branch_color;
            style.accent = branch_color;
        }
    }
```

`render_connectors` 中连接线颜色改为分支色（无分支色回退默认 connector 色）：

```rust
        let connector_color = layout_node
            .branch_index
            .and_then(|index| theme.mindmap.canvas.branch_color(index))
            .unwrap_or(theme.mindmap.canvas.connector);
        draw_connector(
            dl,
            layout_node,
            with_alpha(connector_color, opacity),
            constants.connector_width,
            viewport,
        );
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p textora-markdown --lib mmf::canvas`
Expected: PASS。

- [ ] **Step 5: 提交（需用户确认）**

```bash
git add crates/markdown/src/mmf/canvas.rs
git commit -m "feat(mmf): 节点与连接线按一级分支调色板染色"
```

---

### Task 8: 文本按深度字号渲染

**Files:**
- Modify: `crates/markdown/src/mmf/canvas.rs`（`render_text` :733-776、`render_drag_preview` :847-905）
- Test: `crates/markdown/src/mmf/canvas.rs`（文件内 `mod tests`；若无易测入口则以编译 + 手动验证为主）

**Interfaces:**
- Consumes: Task 3 的 `font_scale_for_depth(depth)`。
- Produces: 屏幕上各级文本字号 = `base × font_scale_for_depth(depth) × zoom`，与 Task 5/6 的布局与命中度量一致。

- [ ] **Step 1: 写失败测试（可测试部分）**

抽一个纯函数便于测试：

```rust
pub(crate) fn node_font_size(base_font_size: f32, depth: u8, zoom: f32, constants: &LayoutConstants) -> f32 {
    base_font_size * constants.font_scale_for_depth(depth) * zoom
}
```

测试：

```rust
    #[test]
    fn node_font_size_scales_with_depth_and_zoom() {
        let constants = LayoutConstants::default();

        let root = node_font_size(14.0, 0, 2.0, &constants);
        let level2 = node_font_size(14.0, 2, 2.0, &constants);
        assert!(root > level2);
        assert_eq!(level2, 14.0 * 1.0 * 2.0);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-markdown --lib mmf::canvas 2>&1 | tail -20`
Expected: 编译失败，函数不存在。

- [ ] **Step 3: 实现**

`render_text` 中：

```rust
        let font_size = node_font_size(shaper.font_size(), ln.depth, viewport.zoom, constants);
```

`render_drag_preview` 中，利用既有 `layout.nodes.iter().find(|(_, node)| layout_rect(node) == preview.source_rect)` 查找同时取出深度（重构该闭包返回 `(style, depth)`），文本字号：

```rust
        let font_size = node_font_size(shaper.font_size(), source_depth, viewport.zoom, constants);
```

`source_depth` 取源节点深度；查找失败（is_valid=false 分支）回退 `0` 档以外的中性深度，直接用 `2`（即 1.0 缩放档）即可，注释说明理由。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p textora-markdown --lib mmf::canvas`
Expected: PASS。

- [ ] **Step 5: 手动冒烟验证**

Run: `cargo run`（或项目惯用启动方式）打开 `test_data/sample.mmap.md`，确认：
- 根/一级/二级字号与卡片高度有明显梯度；
- 各一级分支的连接线与边框颜色不同；
- 双击编辑文本时光标位置与文字对齐（命中几何未漂移）。

- [ ] **Step 6: 提交（需用户确认）**

```bash
git add crates/markdown/src/mmf/canvas.rs
git commit -m "feat(mmf): 文本按深度字号渲染"
```

---

### Task 9: 全量验证

**Files:**
- 无新增修改（如 verify 暴露问题则回到对应 Task 修复）

- [ ] **Step 1: 全量验证**

Run: `./scripts/verify.sh`
Expected: 全绿。

- [ ] **Step 2: 更新文档**

若 `docs/` 下有描述 mmap 主题的既有 spec（`docs/specs/` 内搜 mindmap），补充 `depth_font_scales` 与 `branch_palette` 两个新配置键说明；无则跳过。

## Self-Review 记录

- **Spec 覆盖**:字号分级 → Task 1/3/5/6/8；分支配色 → Task 1/4/7；主题文件配置 → Task 2；优先级约束 → Task 7 测试覆盖。
- **Placeholder 扫描**:Task 4/7 测试中树构造依赖既有测试 helper，已在步骤中注明"先读既有构造模式照搬"——执行者需以仓库实际 helper 名为准，不得自行发明。
- **类型一致性**:`font_scale_for_depth(depth: u8) -> f32`（Task 1 geometry 版与 Task 3 constants 版签名一致）;`branch_color(usize) -> Option<[f32;4]>`;`branch_index: Option<usize>`;`measured_card_width_for_depth(title, constants, shaper, depth)` 在 Task 5 定义、Task 6 复用。
