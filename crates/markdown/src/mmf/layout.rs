use super::model::*;
use super::utils::collect_nodes_dfs;
use crate::grapheme_map::grapheme_byte_boundaries;
use shaping::{GlyphCluster, Shaper};
use std::cmp::Ordering;
use std::ops::Range;

use ui::core::geom::Rect;

const MIN_CARD_WIDTH: f32 = 60.0;
const MAX_CONTROL_HIT_SIZE_DP: f32 = 36.0;
pub const EXPANDED_CONTROL_RIGHT_OFFSET_DP: f32 = 8.0;

pub const EMPTY_TITLE_PLACEHOLDER: &str = "输入主题";

/// 渲染期间替代单个节点标题的纯视觉文字，不改变节点的源码范围。
#[derive(Clone, Copy, Debug)]
pub struct ProjectedTitle<'a> {
    pub node_index: usize,
    pub text: &'a str,
}

/// 布局常量（从 Theme 或默认值读入，非硬编码）
#[derive(Clone, PartialEq, Debug)]
pub struct LayoutConstants {
    pub card_height: f32,
    pub card_padding_x: f32,
    pub card_padding_y: f32,
    pub root_child_gap: f32,
    pub nested_child_gap: f32,
    pub sibling_gap: f32,
    pub card_radius: f32,
    pub connector_width: f32,
    pub expanded_control_right_offset: f32,
    /// 各深度字号缩放，下标即深度（0=根）；来自 theme.mindmap.geometry。
    pub depth_font_scales: Vec<f32>,
}

impl Default for LayoutConstants {
    fn default() -> Self {
        Self::scaled(1.0)
    }
}

impl LayoutConstants {
    pub fn scaled(dpi_scale: f32) -> Self {
        Self {
            card_height: 44.0 * dpi_scale,
            card_padding_x: 20.0 * dpi_scale,
            card_padding_y: 10.0 * dpi_scale,
            root_child_gap: 35.0 * dpi_scale,
            nested_child_gap: 25.0 * dpi_scale,
            sibling_gap: 24.0 * dpi_scale,
            card_radius: 10.0 * dpi_scale,
            connector_width: 8.0 * dpi_scale,
            expanded_control_right_offset: EXPANDED_CONTROL_RIGHT_OFFSET_DP * dpi_scale,
            depth_font_scales: ui::theme::MindmapGeometry::default().depth_font_scales,
        }
    }

    pub fn child_gap_for_parent_depth(&self, parent_depth: u8) -> f32 {
        if parent_depth == 0 { self.root_child_gap } else { self.nested_child_gap }
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

/// 单个节点的布局结果
#[derive(Debug, Clone)]
pub struct LayoutNode {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// 布局数组中的可见节点序号（保留旧字段供现有渲染代码使用）。
    pub node_idx: usize,
    /// 节点在完整 AST DFS 先序遍历中的序号。
    pub source_node_index: usize,
    pub depth: u8,
    pub connector_from: (f32, f32),    // 连线起点（父右边缘中点）
    pub connector_to: (f32, f32),      // 连线终点（本节点左边缘中点）
    pub connector_turn_x: Option<f32>, // 连线转折点的共享 x 轴
    /// 所属一级分支序号（根为 None，根的第 N 个孩子为 Some(N)，更深层继承）。
    pub branch_index: Option<usize>,
}

/// 完整布局结果
pub struct LayoutTree {
    pub nodes: Vec<LayoutNode>, // DFS 序
    /// 按 `(y, x)` 稳定排序的 DFS 节点索引，用于确定性的视口裁剪。
    pub y_sorted_indices: Vec<usize>,
    pub total_w: f32,
    pub total_h: f32,
}

impl LayoutTree {
    /// 返回包含节点、连接线端点及可选中外框的完整内容范围。
    pub fn content_bounds(&self, selection_outline_gap: f32) -> Rect {
        let gap = selection_outline_gap.max(0.0);
        let Some(first_node) = self.nodes.first() else {
            return Rect::ZERO;
        };
        let mut min_x = first_node.x - gap;
        let mut min_y = first_node.y - gap;
        let mut max_x = first_node.x + first_node.w + gap;
        let mut max_y = first_node.y + first_node.h + gap;

        for node in &self.nodes {
            min_x = min_x.min(node.x - gap).min(node.connector_from.0).min(node.connector_to.0);
            min_y = min_y.min(node.y - gap).min(node.connector_from.1).min(node.connector_to.1);
            max_x = max_x
                .max(node.x + node.w + gap)
                .max(node.connector_from.0)
                .max(node.connector_to.0);
            max_y = max_y
                .max(node.y + node.h + gap)
                .max(node.connector_from.1)
                .max(node.connector_to.1);
        }

        Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }

    /// 返回与垂直视口（含缓冲区）相交的节点 DFS 索引，顺序按 `(y, x)` 确定。
    pub fn visible_node_indices(&self, viewport: Rect, buffer: f32) -> Vec<usize> {
        let top = viewport.y - buffer;
        let bottom = viewport.y + viewport.h + buffer;

        self.y_sorted_indices
            .iter()
            .copied()
            .filter(|&index| {
                let node = &self.nodes[index];
                node.y + node.h >= top
                    && node.y <= bottom
                    && node.x + node.w >= viewport.x - buffer
                    && node.x <= viewport.x + viewport.w + buffer
            })
            .collect()
    }
}

/// 返回节点后代数量，不受节点当前展开状态影响。
pub fn descendant_count(node: &Node) -> usize {
    node.children.iter().map(|child| 1 + descendant_count(child)).sum()
}

fn subtree_node_count(node: &Node) -> usize {
    1 + descendant_count(node)
}

fn is_expanded(node: &Node, source_node_index: usize) -> bool {
    source_node_index == 0 || !node.props.as_ref().is_some_and(|props| props.collapsed)
}

/// 字底向上计算当前可见子树高度。
fn subtree_height(
    node: &Node,
    source_node_index: usize,
    depth: u8,
    constants: &LayoutConstants,
) -> f32 {
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

/// 自顶向下分配坐标
fn assign_positions(
    node: &Node,
    source_node_index: usize,
    depth: u8,
    y_offset: f32,
    x: f32,
    parent_connector_from: Option<(f32, f32)>,
    parent_connector_turn_x: Option<f32>,
    branch_index: Option<usize>,
    constants: &LayoutConstants,
    card_widths_by_depth: &[f32],
    out: &mut Vec<LayoutNode>,
) -> usize {
    let card_w = card_width_for_depth(card_widths_by_depth, depth);
    let card_h = constants.card_height_for_depth(depth);
    let sub_h = subtree_height(node, source_node_index, depth, constants);
    let card_y = y_offset + (sub_h - card_h) / 2.0;

    let visible_node_index = out.len();

    // 计算连线端点
    let connector_to = (x, card_y + card_h / 2.0); // 左边缘中点
    let connector_from = parent_connector_from.unwrap_or(connector_to);

    out.push(LayoutNode {
        x,
        y: card_y,
        w: card_w,
        h: card_h,
        node_idx: visible_node_index,
        source_node_index,
        depth,
        connector_from,
        connector_to,
        connector_turn_x: parent_connector_turn_x,
        branch_index,
    });

    if !is_expanded(node, source_node_index) {
        return source_node_index + subtree_node_count(node);
    }

    // 分配子节点
    let this_connector = (x + card_w, card_y + card_h / 2.0);

    // 先计算所有子节点总高度（含 sibling gap），再整体在父节点子树高度内垂直居中。
    let mut children_total_height = 0.0f32;
    let mut child_source_index = source_node_index + 1;
    for child in &node.children {
        children_total_height += subtree_height(child, child_source_index, depth + 1, constants);
        child_source_index += subtree_node_count(child);
    }
    children_total_height += node.children.len().saturating_sub(1) as f32 * constants.sibling_gap;

    let mut cursor = y_offset + (sub_h - children_total_height) / 2.0;
    let mut child_source_index = source_node_index + 1;
    for (child_ordinal, child) in node.children.iter().enumerate() {
        let child_h = subtree_height(child, child_source_index, depth + 1, constants);
        let child_x = x + card_w + constants.child_gap_for_parent_depth(depth);
        let child_connector_turn_x = (this_connector.0 + child_x) * 0.5;
        // 根的孩子各自领取一级分支序号，更深层继承祖先的序号。
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

    child_source_index
}

fn collect_card_widths_by_depth(
    node: &Node,
    depth: u8,
    source_node_index: usize,
    constants: &LayoutConstants,
    shaper: &mut Shaper,
    projected_title: Option<ProjectedTitle<'_>>,
    out: &mut Vec<f32>,
) -> usize {
    let depth_idx = depth as usize;
    if out.len() <= depth_idx {
        out.resize(depth_idx + 1, MIN_CARD_WIDTH);
    }

    let title = projected_title
        .filter(|projected| projected.node_index == source_node_index)
        .map(|projected| projected.text)
        .unwrap_or_else(|| title_or_placeholder(&node.title));
    let card_w = measured_card_width_for_depth(title, constants, shaper, depth);
    out[depth_idx] = out[depth_idx].max(card_w);

    if !is_expanded(node, source_node_index) {
        return source_node_index + subtree_node_count(node);
    }

    let mut child_source_index = source_node_index + 1;
    for child in &node.children {
        child_source_index = collect_card_widths_by_depth(
            child,
            depth + 1,
            child_source_index,
            constants,
            shaper,
            projected_title,
            out,
        );
    }

    child_source_index
}

fn title_or_placeholder(title: &str) -> &str {
    if title.is_empty() { EMPTY_TITLE_PLACEHOLDER } else { title }
}

pub(crate) fn measured_card_width(
    title: &str,
    constants: &LayoutConstants,
    shaper: &mut Shaper,
) -> f32 {
    (measure_text(title, shaper) + 2.0 * constants.card_padding_x).max(MIN_CARD_WIDTH)
}

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

fn card_width_for_depth(card_widths_by_depth: &[f32], depth: u8) -> f32 {
    card_widths_by_depth.get(depth as usize).copied().unwrap_or(MIN_CARD_WIDTH)
}

fn measure_text(text: &str, shaper: &mut Shaper) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    match shaper.shape(text) {
        Ok(run) => run.width,
        Err(_) => text.len() as f32 * shaper.font_size() * 0.5, // fallback
    }
}

pub fn compute_layout(
    tree: &Tree,
    shaper: &mut Shaper,
    constants: &LayoutConstants,
    projected_title: Option<ProjectedTitle<'_>>,
) -> LayoutTree {
    let mut nodes = Vec::new();
    let mut card_widths_by_depth = Vec::new();
    collect_card_widths_by_depth(
        &tree.root,
        0,
        0,
        constants,
        shaper,
        projected_title,
        &mut card_widths_by_depth,
    );
    assign_positions(
        &tree.root,
        0,
        0,
        0.0,
        0.0,
        None,
        None,
        None,
        constants,
        &card_widths_by_depth,
        &mut nodes,
    );
    let total_h = subtree_height(&tree.root, 0, 0, constants);
    let total_w = nodes.iter().map(|node| node.x + node.w).max_by(f32::total_cmp).unwrap_or(0.0);
    let mut y_sorted_indices: Vec<usize> = (0..nodes.len()).collect();
    y_sorted_indices.sort_by(|left, right| {
        nodes[*left]
            .y
            .total_cmp(&nodes[*right].y)
            .then_with(|| nodes[*left].x.total_cmp(&nodes[*right].x))
    });

    LayoutTree { nodes, y_sorted_indices, total_w, total_h }
}

/// 单个节点的精确命中几何，所有标题位置均对齐到 Unicode grapheme 边界。
pub struct NodeHitGeometry {
    pub source_node_index: usize,
    pub card_rect: Rect,
    pub title_rect: Rect,
    /// 标题内每个 grapheme 起点及 one-past-end sentinel 的 UTF-8 字节偏移。
    pub grapheme_byte_offsets: Vec<usize>,
    /// 与 `grapheme_byte_offsets` 一一对应的画布 x 边缘。
    pub grapheme_edges: Vec<f32>,
    pub title_byte_range: Range<usize>,
    pub subtree_source_range: Range<usize>,
}

/// 折叠/展开控件的命中几何。
pub struct ControlHitGeometry {
    pub source_node_index: usize,
    pub bounds: Rect,
}

/// 标题命中边界的容差（画布像素），吸收屏幕↔画布坐标往返转换的浮点误差，
/// 避免恰好点中标题边缘的点击穿透到卡片层。
const TITLE_HIT_EDGE_TOLERANCE_PX: f32 = 0.01;

impl NodeHitGeometry {
    pub fn contains_title(&self, px: f32, py: f32) -> bool {
        px >= self.title_rect.left() - TITLE_HIT_EDGE_TOLERANCE_PX
            && px <= self.title_rect.right() + TITLE_HIT_EDGE_TOLERANCE_PX
            && py >= self.title_rect.top()
            && py < self.title_rect.bottom()
    }
}

/// 节点命中几何，按 DFS 索引排列。
pub struct HitMap {
    pub nodes: Vec<NodeHitGeometry>,
    pub controls: Vec<ControlHitGeometry>,
    /// 过渡期只读兼容字段；Task7 会改为消费 `nodes` 后移除。
    pub node_rects: Vec<Rect>,
    /// 过渡期只读兼容字段；其值按 grapheme 边缘生成，不能用作新的编辑命中数据。
    pub title_char_edges: Vec<Vec<f32>>,
}

pub fn build_hit_map(
    tree: &Tree,
    layout: &LayoutTree,
    shaper: &mut Shaper,
    constants: &LayoutConstants,
    projected_title: Option<ProjectedTitle<'_>>,
) -> HitMap {
    let n = layout.nodes.len();
    let mut hit_nodes = Vec::with_capacity(n);
    let mut node_rects = Vec::with_capacity(n);
    let mut title_char_edges = Vec::with_capacity(n);

    // DFS 收集节点引用；布局只保留可见节点，因此必须按完整 DFS 序查找。
    let nodes = collect_nodes_dfs(&tree.root);

    for ln in &layout.nodes {
        let Some(node) = nodes.get(ln.source_node_index) else {
            continue;
        };
        // 命中几何必须与渲染（Task 8）使用同一按深度缩放的字号，否则点击/光标会漂移。
        let base_size = shaper.font_size();
        shaper.set_font_size(base_size * constants.font_scale_for_depth(ln.depth));
        let text_x = ln.x + constants.card_padding_x;
        let title = projected_title
            .filter(|projected| projected.node_index == ln.source_node_index)
            .map(|projected| projected.text)
            .unwrap_or_else(|| title_or_placeholder(&node.title));
        let grapheme_byte_offsets = grapheme_byte_boundaries(title);
        let grapheme_edges = grapheme_edges(title, &grapheme_byte_offsets, text_x, shaper);
        shaper.set_font_size(base_size);
        let card_rect = Rect::new(ln.x, ln.y, ln.w, ln.h);
        let title_end = grapheme_edges.last().copied().unwrap_or(text_x);
        let title_rect = Rect::new(
            text_x,
            ln.y + constants.card_padding_y,
            (title_end - text_x).max(0.0),
            ln.h - 2.0 * constants.card_padding_y,
        );

        node_rects.push(card_rect);
        title_char_edges.push(legacy_title_char_edges(
            title,
            &grapheme_byte_offsets,
            &grapheme_edges,
        ));
        hit_nodes.push(NodeHitGeometry {
            source_node_index: ln.source_node_index,
            card_rect,
            title_rect,
            grapheme_byte_offsets,
            grapheme_edges,
            title_byte_range: node.title_byte_range.clone(),
            subtree_source_range: node.subtree_source_range.clone(),
        });
    }

    let controls = build_control_hit_geometries(layout, &nodes, constants);

    HitMap { nodes: hit_nodes, controls, node_rects, title_char_edges }
}

fn build_control_hit_geometries(
    layout: &LayoutTree,
    source_nodes: &[&Node],
    constants: &LayoutConstants,
) -> Vec<ControlHitGeometry> {
    let mut controls = Vec::new();
    for (visible_index, layout_node) in layout.nodes.iter().enumerate() {
        let Some(node) = source_nodes.get(layout_node.source_node_index) else {
            continue;
        };
        if layout_node.source_node_index == 0 || node.children.is_empty() {
            continue;
        }

        let child_gap = constants.child_gap_for_parent_depth(layout_node.depth);
        let control_size = MAX_CONTROL_HIT_SIZE_DP.min(child_gap);
        if control_size.partial_cmp(&0.0) != Some(Ordering::Greater) {
            continue;
        }

        let control_turn_x = layout
            .nodes
            .get(visible_index + 1)
            .filter(|first_child| {
                first_child.source_node_index == layout_node.source_node_index + 1
            })
            .and_then(|first_child| first_child.connector_turn_x)
            .unwrap_or(
                layout_node.x
                    + layout_node.w
                    + constants.child_gap_for_parent_depth(layout_node.depth) * 0.5,
            );
        let center_y = layout_node.y + layout_node.h / 2.0;
        let half_size = control_size / 2.0;
        controls.push(ControlHitGeometry {
            source_node_index: layout_node.source_node_index,
            bounds: Rect::new(
                control_turn_x - half_size,
                center_y - half_size,
                control_size,
                control_size,
            ),
        });
    }
    controls
}

fn grapheme_edges(
    title: &str,
    grapheme_byte_offsets: &[usize],
    text_x: f32,
    shaper: &mut Shaper,
) -> Vec<f32> {
    if let Ok(shaped) = shaper.shape(title) {
        return grapheme_edges_from_shaped_clusters(
            grapheme_byte_offsets,
            text_x,
            &shaped.clusters,
        );
    }

    grapheme_edges_fallback(title, grapheme_byte_offsets, text_x, shaper)
}

fn grapheme_edges_from_shaped_clusters(
    grapheme_byte_offsets: &[usize],
    text_x: f32,
    clusters: &[GlyphCluster],
) -> Vec<f32> {
    let mut advances = vec![0.0; grapheme_byte_offsets.len().saturating_sub(1)];
    for cluster in clusters {
        if let Ok(grapheme_index) = grapheme_byte_offsets.binary_search(&cluster.byte_range.start)
            && let Some(advance) = advances.get_mut(grapheme_index)
        {
            *advance += cluster.advance;
        }
    }

    let mut edges = Vec::with_capacity(grapheme_byte_offsets.len());
    let mut x = text_x;
    edges.push(x);

    for advance in advances {
        x += advance;
        edges.push(x);
    }

    edges
}

fn grapheme_edges_fallback(
    title: &str,
    grapheme_byte_offsets: &[usize],
    text_x: f32,
    shaper: &mut Shaper,
) -> Vec<f32> {
    let mut edges = Vec::with_capacity(grapheme_byte_offsets.len());
    let mut x = text_x;
    edges.push(x);

    for byte_range in grapheme_byte_offsets.windows(2) {
        let grapheme = &title[byte_range[0]..byte_range[1]];
        let advance =
            shaper.grapheme_advance(grapheme).unwrap_or_else(|_| shaper.font_size() * 0.5);
        x += advance;
        edges.push(x);
    }

    edges
}

fn legacy_title_char_edges(
    title: &str,
    grapheme_byte_offsets: &[usize],
    grapheme_edges: &[f32],
) -> Vec<f32> {
    if title.is_empty() {
        return grapheme_edges.first().copied().into_iter().collect();
    }

    title
        .char_indices()
        .map(|(byte_offset, _)| {
            let grapheme_end_index = match grapheme_byte_offsets.binary_search(&byte_offset) {
                Ok(grapheme_start_index) => grapheme_start_index + 1,
                Err(next_grapheme_start_index) => next_grapheme_start_index,
            };
            grapheme_edges[grapheme_end_index]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mmf::parser;

    fn dummy_tree() -> Tree {
        parser::parse("# Root\n\n## Child1\n\n## Child2\n\n### GrandChild\n").unwrap()
    }

    #[test]
    fn layout_root_is_at_origin() {
        let tree = dummy_tree();
        let constants = LayoutConstants::default();
        let shaper = &mut Shaper::new().expect("shaper");
        let lt = compute_layout(&tree, shaper, &constants, None);
        assert!((lt.nodes[0].x - 0.0).abs() < 1.0, "root x should be 0");
    }

    #[test]
    fn child_is_indented_right() {
        let tree = dummy_tree();
        let constants = LayoutConstants::default();
        let shaper = &mut Shaper::new().expect("shaper");
        let lt = compute_layout(&tree, shaper, &constants, None);
        let child = &lt.nodes[1]; // "Child1"
        assert!(
            child.x > lt.nodes[0].x + 50.0,
            "child should be to the right of root, child.x={}, root.x={}",
            child.x,
            lt.nodes[0].x
        );
    }

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

    #[test]
    fn wide_parent_keeps_grandchild_connector_pointing_right() {
        let tree = parser::parse(
            "# Root\n## A parent title that is deliberately wider than one level indent\n### Child\n",
        )
        .expect("fixture must be valid MMF");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let layout = compute_layout(&tree, &mut shaper, &LayoutConstants::default(), None);
        let parent = &layout.nodes[1];
        let child = &layout.nodes[2];

        assert!(
            child.x > parent.x + parent.w,
            "child card must be right of its parent: parent_right={}, child_left={}",
            parent.x + parent.w,
            child.x,
        );
        assert!(
            child.connector_from.0 < child.connector_to.0,
            "tapered parent-to-child connector must point right",
        );
    }

    #[test]
    fn sibling_connectors_share_their_parent_turn_axis() {
        let tree = parser::parse("# Root\n## First\n## Second\n## Third\n")
            .expect("fixture must be valid MMF");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let layout = compute_layout(&tree, &mut shaper, &LayoutConstants::default(), None);
        let root = &layout.nodes[0];
        let children = &layout.nodes[1..4];
        let turn_x = children[0].connector_turn_x.expect("child connector must have a turn axis");

        assert!(root.connector_turn_x.is_none(), "root must not have an incoming connector axis");
        assert!(turn_x > children[0].connector_from.0);
        assert!(turn_x < children[0].connector_to.0);
        assert!(children.iter().all(|child| child.connector_turn_x == Some(turn_x)));
    }

    #[test]
    fn siblings_are_stacked_vertically() {
        let tree = dummy_tree();
        let constants = LayoutConstants::default();
        let shaper = &mut Shaper::new().expect("shaper");
        let lt = compute_layout(&tree, shaper, &constants, None);
        // Child1 在 Child2 上面
        let child1_y = lt.nodes[1].y;
        let child2_y = lt.nodes[2].y;
        assert!(
            child2_y > child1_y + 10.0,
            "siblings should be stacked, child1.y={}, child2.y={}",
            child1_y,
            child2_y
        );
    }

    #[test]
    fn scaled_constants_make_mindmap_less_dense() {
        let constants = LayoutConstants::scaled(2.0);
        assert_eq!(constants.card_height, 88.0);
        assert_eq!(constants.card_padding_x, 40.0);
        assert_eq!(constants.card_padding_y, 20.0);
        assert_eq!(constants.root_child_gap, 70.0);
        assert_eq!(constants.nested_child_gap, 50.0);
        assert_eq!(constants.sibling_gap, 48.0);
        assert_eq!(constants.card_radius, 20.0);
        assert_eq!(constants.connector_width, 16.0);
        assert_eq!(constants.expanded_control_right_offset, 16.0);
    }

    #[test]
    fn nodes_at_same_depth_share_card_width() {
        let tree = parser::parse(
            "# Root\n\n## A\n\n### Tiny\n\n## Much longer second level\n\n### Much longer third level title\n",
        )
        .unwrap();
        let constants = LayoutConstants::default();
        let shaper = &mut Shaper::new().expect("shaper");
        let lt = compute_layout(&tree, shaper, &constants, None);

        let mut widths_by_depth = std::collections::BTreeMap::<u8, Vec<f32>>::new();
        for node in &lt.nodes {
            widths_by_depth.entry(node.depth).or_default().push(node.w);
        }

        for (depth, widths) in widths_by_depth {
            if widths.len() < 2 {
                continue;
            }
            let first_width = widths[0];
            assert!(
                widths.iter().all(|width| (width - first_width).abs() < 0.01),
                "cards at depth {depth} should share width: {:?}",
                widths
            );
        }
    }

    #[test]
    fn hit_geometry_uses_grapheme_byte_boundaries() {
        let tree =
            parser::parse("# A👨\u{200D}👩\u{200D}👧中\n").expect("mindmap source should parse");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let constants = LayoutConstants::default();
        let layout = compute_layout(&tree, &mut shaper, &constants, None);
        let hit_map = build_hit_map(&tree, &layout, &mut shaper, &constants, None);
        let geometry = &hit_map.nodes[0];

        assert_eq!(geometry.grapheme_byte_offsets, vec![0, 1, 19, 22]);
        assert_eq!(geometry.grapheme_byte_offsets.first(), Some(&0));
        assert_eq!(geometry.grapheme_byte_offsets.last(), Some(&tree.root.title.len()));
        assert_eq!(geometry.grapheme_byte_offsets.len(), geometry.grapheme_edges.len());
    }

    #[test]
    fn grapheme_edges_match_whole_title_shaping_for_latin_kerning() {
        let title = "从toB做起";
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let edges = grapheme_edges(title, &grapheme_byte_boundaries(title), 0.0, &mut shaper);
        let shaped = shaper.shape(title).expect("test title should shape");
        let expected_width: f32 = shaped.clusters.iter().map(|cluster| cluster.advance).sum();

        assert!((edges.last().copied().unwrap_or_default() - expected_width).abs() < 0.01);
    }

    #[test]
    fn empty_title_measures_placeholder_and_uses_visual_graphemes() {
        let tree = parser::parse("# Root\n##\n").expect("mindmap source should parse");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let constants = LayoutConstants::default();
        let layout = compute_layout(&tree, &mut shaper, &constants, None);
        let hit_map = build_hit_map(&tree, &layout, &mut shaper, &constants, None);

        assert!(layout.nodes[1].w > MIN_CARD_WIDTH);
        assert_eq!(
            hit_map.nodes[1].grapheme_byte_offsets.last(),
            Some(&EMPTY_TITLE_PLACEHOLDER.len())
        );
    }

    #[test]
    fn visible_indices_exclude_nodes_outside_viewport() {
        let tree = dummy_tree();
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let layout = compute_layout(&tree, &mut shaper, &LayoutConstants::default(), None);
        let visible = layout.visible_node_indices(Rect::new(0.0, 0.0, 400.0, 50.0), 0.0);

        assert!(visible.len() < layout.nodes.len());
        assert!(visible.iter().all(|&index| {
            let node = &layout.nodes[index];
            node.y + node.h >= 0.0 && node.y <= 50.0
        }));
        assert!(visible.windows(2).all(|pair| {
            let left = &layout.nodes[pair[0]];
            let right = &layout.nodes[pair[1]];
            (left.y, left.x) <= (right.y, right.x)
        }));
    }

    #[test]
    fn content_bounds_include_negative_connectors_and_selection_outline() {
        let layout = LayoutTree {
            nodes: vec![LayoutNode {
                x: 10.0,
                y: 20.0,
                w: 30.0,
                h: 40.0,
                node_idx: 0,
                source_node_index: 0,
                depth: 0,
                connector_from: (-50.0, 100.0),
                connector_to: (-20.0, -30.0),
                connector_turn_x: None,
                branch_index: None,
            }],
            y_sorted_indices: vec![0],
            total_w: 40.0,
            total_h: 60.0,
        };

        assert_eq!(layout.content_bounds(5.0), Rect::new(-50.0, -30.0, 95.0, 130.0));
    }

    #[test]
    fn collapsed_node_hides_all_descendants_but_retains_original_dfs_index() {
        let tree =
            parser::parse("# Root\n## A\n```toml node\ncollapsed = true\n```\n### B\n## C\n")
                .expect("fixture must parse");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let layout = compute_layout(&tree, &mut shaper, &LayoutConstants::default(), None);

        assert_eq!(
            layout.nodes.iter().map(|node| node.source_node_index).collect::<Vec<_>>(),
            vec![0, 1, 3]
        );
        assert!(layout.nodes.iter().all(|node| node.source_node_index != 2));
    }

    #[test]
    fn descendant_count_includes_all_nested_descendants() {
        let tree =
            parser::parse("# Root\n## A\n### B\n#### C\n## D\n").expect("fixture must parse");

        assert_eq!(descendant_count(&tree.root), 4);
        assert_eq!(descendant_count(&tree.root.children[0]), 2);
        assert_eq!(descendant_count(&tree.root.children[0].children[0]), 1);
    }

    #[test]
    fn collapsed_card_width_matches_expanded_title_width() {
        let collapsed = parser::parse(
            "# Root\n## Child\n```toml node\ncollapsed = true\n```\n### A\n#### B\n### C\n",
        )
        .expect("fixture must parse");
        let expanded =
            parser::parse("# Root\n## Child\n### A\n#### B\n### C\n").expect("fixture must parse");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let constants = LayoutConstants::default();
        let collapsed_layout = compute_layout(&collapsed, &mut shaper, &constants, None);
        let expanded_layout = compute_layout(&expanded, &mut shaper, &constants, None);
        let collapsed_child = collapsed_layout
            .nodes
            .iter()
            .find(|node| node.source_node_index == 1)
            .expect("collapsed child layout");
        let expanded_child = expanded_layout
            .nodes
            .iter()
            .find(|node| node.source_node_index == 1)
            .expect("expanded child layout");
        assert!(
            (collapsed_child.w - expanded_child.w).abs() < f32::EPSILON,
            "collapsed title must not reserve descendant-count width"
        );
    }

    #[test]
    fn controls_exclude_root_and_leaves_and_use_shared_child_turn_point() {
        let tree = parser::parse("# Root\n## Branch\n### First\n### Second\n## Leaf\n")
            .expect("fixture must parse");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let constants = LayoutConstants::default();
        let layout = compute_layout(&tree, &mut shaper, &constants, None);
        let hit_map = build_hit_map(&tree, &layout, &mut shaper, &constants, None);

        assert_eq!(
            hit_map.controls.iter().map(|control| control.source_node_index).collect::<Vec<_>>(),
            vec![1]
        );
        let branch_layout =
            layout.nodes.iter().find(|node| node.source_node_index == 1).expect("branch layout");
        let first_child_layout = layout
            .nodes
            .iter()
            .find(|node| node.source_node_index == 2)
            .expect("first child layout");
        let control = &hit_map.controls[0];
        assert_eq!(control.bounds.w, constants.nested_child_gap);
        assert_eq!(control.bounds.h, constants.nested_child_gap);
        assert!(
            (control.bounds.x + control.bounds.w / 2.0
                - first_child_layout.connector_turn_x.unwrap())
            .abs()
                < 0.01
        );
        assert!(
            (control.bounds.y + control.bounds.h / 2.0 - (branch_layout.y + branch_layout.h / 2.0))
                .abs()
                < 0.01
        );
    }

    #[test]
    fn nested_control_bounds_fit_between_its_parent_and_child_cards() {
        let tree = parser::parse("# Root\n## Branch\n### Child\n").expect("fixture must parse");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let constants = LayoutConstants::default();
        let layout = compute_layout(&tree, &mut shaper, &constants, None);
        let hit_map = build_hit_map(&tree, &layout, &mut shaper, &constants, None);
        let branch = &layout.nodes[1];
        let child = &layout.nodes[2];
        let control = hit_map.controls.first().expect("branch control");

        assert_eq!(control.bounds.w, constants.nested_child_gap);
        assert_eq!(control.bounds.h, constants.nested_child_gap);
        assert!(control.bounds.x >= branch.x + branch.w);
        assert!(control.bounds.right() <= child.x);
    }

    #[test]
    fn zero_width_child_gap_does_not_create_a_control() {
        let tree = parser::parse("# Root\n## Branch\n### Child\n").expect("fixture must parse");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let constants = LayoutConstants { nested_child_gap: 0.0, ..LayoutConstants::default() };
        let layout = compute_layout(&tree, &mut shaper, &constants, None);
        let hit_map = build_hit_map(&tree, &layout, &mut shaper, &constants, None);

        assert!(hit_map.controls.is_empty());
    }

    #[test]
    fn collapsed_node_expand_control_stays_on_its_connector_turn_axis_at_high_dpi() {
        const DPI_SCALE: f32 = 2.0;
        let tree = parser::parse(
            "# Root\n## Branch\n```toml node\ncollapsed = true\n```\n### First\n### Second\n",
        )
        .expect("fixture must parse");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let constants = LayoutConstants::scaled(DPI_SCALE);
        let layout = compute_layout(&tree, &mut shaper, &constants, None);
        let hit_map = build_hit_map(&tree, &layout, &mut shaper, &constants, None);
        let branch =
            layout.nodes.iter().find(|node| node.source_node_index == 1).expect("branch layout");
        let control = hit_map.controls.first().expect("branch control");
        let control_center_x = control.bounds.x + control.bounds.w / 2.0;
        let connector_turn_x = branch.x + branch.w + constants.nested_child_gap / 2.0;

        assert!((control_center_x - connector_turn_x).abs() < 0.01);
    }

    #[test]
    fn card_height_scales_with_depth_font_scale() {
        let constants = LayoutConstants::default();

        let root_h = constants.card_height_for_depth(0);
        let level2_h = constants.card_height_for_depth(2);
        assert!(root_h > level2_h, "root card must be taller than level-2 card");
        assert_eq!(level2_h, constants.card_height);
        assert_eq!(constants.card_height_for_depth(9), constants.card_height_for_depth(3));
    }

    #[test]
    fn branch_index_tracks_top_level_ancestor() {
        // 构造: root ─┬─ A ── A1
        //            └─ B
        // DFS 序：root=0, A=1, A1=2, B=3
        let tree = parser::parse("# Root\n\n## A\n\n### A1\n\n## B\n").expect("fixture must parse");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let layout = compute_layout(&tree, &mut shaper, &LayoutConstants::default(), None);

        assert_eq!(layout.nodes.len(), 4);
        assert_eq!(layout.nodes[0].branch_index, None);
        assert_eq!(layout.nodes[1].branch_index, Some(0));
        assert_eq!(layout.nodes[2].branch_index, Some(0));
        assert_eq!(layout.nodes[3].branch_index, Some(1));
    }

    #[test]
    fn layout_assigns_taller_cards_to_shallower_depths() {
        let tree =
            parser::parse("# Root\n\n## Child\n\n### GrandChild\n").expect("fixture must parse");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let constants = LayoutConstants::default();
        let layout = compute_layout(&tree, &mut shaper, &constants, None);

        assert_eq!(layout.nodes[0].h, constants.card_height_for_depth(0));
        assert_eq!(layout.nodes[1].h, constants.card_height_for_depth(1));
        assert_eq!(layout.nodes[2].h, constants.card_height_for_depth(2));
        assert!(layout.nodes[0].h > layout.nodes[2].h);
        // 连线终点应落在本卡片左边缘中点
        let node = &layout.nodes[1];
        assert_eq!(node.connector_to.1, node.y + node.h / 2.0);
    }

    #[test]
    fn wider_font_measures_wider_card_for_shallower_depth() {
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let constants = LayoutConstants::default();
        let title = "主题";

        let root_w = measured_card_width_for_depth(title, &constants, &mut shaper, 0);
        let level2_w = measured_card_width_for_depth(title, &constants, &mut shaper, 2);
        assert!(root_w > level2_w, "root_w={root_w}, level2_w={level2_w}");
    }

    #[test]
    fn hit_map_grapheme_edges_match_depth_scaled_text_width() {
        let tree = parser::parse("# 根标题\n\n## 子节点\n").expect("fixture must parse");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let constants = LayoutConstants::default();
        let layout = compute_layout(&tree, &mut shaper, &constants, None);
        let hit_map = build_hit_map(&tree, &layout, &mut shaper, &constants, None);

        let root_hit = &hit_map.nodes[0];
        let edges = &root_hit.grapheme_edges;
        let measured_span =
            edges.last().expect("edges sentinel") - edges.first().expect("edges start");
        // 与测宽路径独立复算：grapheme 边缘跨度 ≈ 按深度缩放字号测量的文本宽
        let mut verify_shaper = Shaper::new().expect("test shaper should initialize");
        let expected = measured_card_width_for_depth("根标题", &constants, &mut verify_shaper, 0)
            - 2.0 * constants.card_padding_x;
        assert!(
            (measured_span - expected).abs() < 1.0,
            "grapheme edges must use depth-scaled font: span={measured_span}, expected={expected}"
        );
    }

    #[test]
    fn expanded_node_collapse_control_stays_on_its_connector_turn_axis() {
        let tree = parser::parse("# Root\n## Branch\n### First\n### Second\n")
            .expect("fixture must parse");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let constants = LayoutConstants::default();
        let layout = compute_layout(&tree, &mut shaper, &constants, None);
        let hit_map = build_hit_map(&tree, &layout, &mut shaper, &constants, None);
        let first_child = layout
            .nodes
            .iter()
            .find(|node| node.source_node_index == 2)
            .expect("first child layout");
        let control = hit_map.controls.first().expect("branch control");
        let control_center_x = control.bounds.x + control.bounds.w / 2.0;
        let expected_turn_x = first_child.connector_turn_x.expect("child connector turn axis");

        assert!((control_center_x - expected_turn_x).abs() < 0.01);
    }

    #[test]
    fn single_child_is_vertically_centered_with_parent() {
        let tree = parser::parse("# Root\n## Child\n").expect("fixture must parse");
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let constants = LayoutConstants::default();
        let layout = compute_layout(&tree, &mut shaper, &constants, None);

        let root = &layout.nodes[0];
        let child = &layout.nodes[1];
        let root_center_y = root.y + root.h / 2.0;
        let child_center_y = child.y + child.h / 2.0;

        assert!(
            (root_center_y - child_center_y).abs() < 0.01,
            "single child should be vertically centered with parent: root_center_y={}, child_center_y={}",
            root_center_y,
            child_center_y
        );
    }
}
