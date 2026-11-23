use std::collections::HashMap;
use std::ops::Range;

/// 思维导图 AST——MMF 源码的结构化投影（只读）。
#[derive(Debug, Clone)]
pub struct Tree {
    pub version: u32,
    pub root: Node,
    pub global_props: HashMap<String, String>,
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
