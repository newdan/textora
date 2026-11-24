use std::collections::HashMap;
use std::ops::Range;

/// Position in the complete source tree's DFS order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceNodeIdx(pub usize);

/// 思维导图 AST——MMF 源码的结构化投影（只读）。
#[derive(Debug, Clone)]
pub struct Tree {
    pub version: u32,
    root: Node,
    pub global_props: HashMap<String, String>,
    pub global_property_source: Option<GlobalPropertySource>,
    node_index: Vec<NodePosition>,
}

#[derive(Debug, Clone)]
struct NodePosition {
    parent: Option<usize>,
    sibling_slot: usize,
    subtree_end: usize,
}

impl Tree {
    pub(super) fn new(
        root: Node,
        global_props: HashMap<String, String>,
        global_property_source: Option<GlobalPropertySource>,
    ) -> Self {
        let mut node_index = Vec::new();
        index_subtree(&root, None, 0, &mut node_index);
        Self { version: 1, root, global_props, global_property_source, node_index }
    }

    pub fn root(&self) -> &Node {
        &self.root
    }

    pub(crate) fn node(&self, index: usize) -> Option<&Node> {
        let position = self.node_index.get(index)?;
        let Some(parent) = position.parent else {
            return Some(self.root());
        };
        self.node(parent)?.children.get(position.sibling_slot)
    }

    pub(crate) fn nodes(&self) -> impl DoubleEndedIterator<Item = &Node> + ExactSizeIterator {
        self.nodes_in(0..self.node_index.len())
    }

    pub(crate) fn subtree_nodes(&self, index: usize) -> impl ExactSizeIterator<Item = &Node> {
        self.nodes_in(self.subtree_indices(index).unwrap_or_default())
    }

    fn nodes_in(
        &self,
        indices: Range<usize>,
    ) -> impl DoubleEndedIterator<Item = &Node> + ExactSizeIterator {
        indices
            .map(|index| self.node(index).expect("DFS index entries refer to immutable tree nodes"))
    }

    pub(crate) fn subtree_index(&self, range: &Range<usize>) -> Option<usize> {
        self.nodes().position(|node| &node.subtree_source_range == range)
    }

    pub(crate) fn parent_index(&self, index: usize) -> Option<usize> {
        self.node_index.get(index)?.parent
    }

    pub(crate) fn subtree_indices(&self, index: usize) -> Option<Range<usize>> {
        Some(index..self.node_index.get(index)?.subtree_end)
    }

    pub(crate) fn child_indices(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        let descendants = self
            .subtree_indices(index)
            .map_or(index..index, |range| range.start.saturating_add(1)..range.end);
        let first_child = (!descendants.is_empty()).then_some(descendants.start);
        std::iter::successors(first_child, move |&child| {
            let next = self.node_index[child].subtree_end;
            (next < descendants.end).then_some(next)
        })
    }

    pub(crate) fn sibling_slot(&self, index: usize) -> Option<usize> {
        let position = self.node_index.get(index)?;
        position.parent.map(|_| position.sibling_slot)
    }
}

fn index_subtree(
    node: &Node,
    parent: Option<usize>,
    sibling_slot: usize,
    index: &mut Vec<NodePosition>,
) {
    let node_index = index.len();
    index.push(NodePosition { parent, sibling_slot, subtree_end: node_index + 1 });
    for (slot, child) in node.children.iter().enumerate() {
        index_subtree(child, Some(node_index), slot, index);
    }
    index[node_index].subtree_end = index.len();
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MindmapFocus {
    None,
    NodeSelected { node_index: usize },
    TitleEditing { node_index: usize, cursor_byte: usize },
    TitleTextSelected { node_index: usize, range: Range<usize> },
}

#[derive(Debug, Clone)]
pub struct Node {
    pub title: String,
    pub children: Vec<Node>,
    pub props: Option<NodeProps>,
    pub note: Option<String>,
    /// 此节点在源码中的完整字节范围
    pub source_range: Range<usize>,
    /// 此节点及其所有后代在源码中的完整字节范围
    pub subtree_source_range: Range<usize>,
    /// 标题文字在源码中的字节范围（不含 `# ` 前缀和换行符）
    pub title_byte_range: Range<usize>,
    /// 标题行中 `#` 标记的字节范围
    pub heading_marker_range: Range<usize>,
    /// 插入第一个子节点的源码字节位置
    pub child_insertion_byte: usize,
    /// `#` 的个数：1=根, 2=一级子, ...
    pub heading_level: u8,
    /// 此节点属性块在源码中的范围及 `collapsed` 值范围
    pub property_source: Option<NodePropertySource>,
    /// 标题行末尾（不含换行符）的源码字节位置
    pub heading_source_end: usize,
}

/// 节点属性块及 `collapsed` 字段在源码中的精确范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodePropertySource {
    pub body_range: Range<usize>,
    pub collapsed_value_range: Option<Range<usize>>,
}

/// 全局 TOML 属性块及 `theme` 字段在源码中的精确范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalPropertySource {
    pub body_range: Range<usize>,
    pub theme_value_range: Option<Range<usize>>,
}

#[derive(Debug, Clone)]
pub struct NodeProps {
    pub id: Option<String>,
    pub priority: Option<String>,
    pub status: Option<String>,
    pub owner: Option<String>,
    pub collapsed: bool,
    pub tags: Vec<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    EmptyDocument,
    MultipleRoots,
    InvalidToml,
    HeadingLevelSkip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MmfDiagnostic {
    pub kind: ParseErrorKind,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl MmfDiagnostic {
    pub(crate) fn new(kind: ParseErrorKind, line: usize, column: usize) -> Self {
        let message = match kind {
            ParseErrorKind::EmptyDocument => "MMF document is empty.",
            ParseErrorKind::MultipleRoots => "MMF document must contain exactly one root heading.",
            ParseErrorKind::InvalidToml => "MMF TOML metadata is invalid.",
            ParseErrorKind::HeadingLevelSkip => "MMF heading levels must form a valid tree.",
        };
        Self { kind, line, column, message: message.to_string() }
    }
}
