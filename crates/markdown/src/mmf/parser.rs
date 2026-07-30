use super::model::*;
use std::collections::HashMap;
use std::ops::Range;

const ROOT_HEADING_LEVEL: u8 = 1;
const MINIMUM_FENCE_LENGTH: usize = 3;
const TRIPLE_QUOTE_LENGTH: usize = 3;

pub fn parse(source: &str) -> Result<Tree, MmfDiagnostic> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() || source.trim().is_empty() {
        return Err(diagnostic_at(source, 0, ParseErrorKind::EmptyDocument));
    }
    let mut cursor = Cursor { lines, idx: 0 };

    // 1. 全局属性
    let global_props_result = parse_global_props(source, &mut cursor)?;

    // 2. 栈式解析 heading 树
    let root = parse_heading_tree(source, &mut cursor)?;
    Ok(Tree { version: 1, root, global_props: global_props_result.properties })
}

struct Cursor<'a> {
    lines: Vec<&'a str>,
    idx: usize,
}

struct GlobalPropsResult {
    properties: HashMap<String, String>,
    property_source: Option<GlobalPropertySource>,
}

fn parse_global_props(source: &str, c: &mut Cursor) -> Result<GlobalPropsResult, MmfDiagnostic> {
    let save = c.idx;
    while c.idx < c.lines.len() {
        let line = c.lines[c.idx];
        if line.trim() == "```toml mindmap" {
            let fence_start = line_start(source, line);
            c.idx += 1;
            let toml_block = collect_toml_block(source, c)
                .ok_or_else(|| diagnostic_at(source, fence_start, ParseErrorKind::InvalidToml))?;
            let toml::Value::Table(table) = toml_block
                .source
                .parse::<toml::Value>()
                .map_err(|_| diagnostic_at(source, fence_start, ParseErrorKind::InvalidToml))?
            else {
                return Err(diagnostic_at(source, fence_start, ParseErrorKind::InvalidToml));
            };
            let mut properties = HashMap::new();
            for (key, value) in &table {
                if key == "theme" && value.as_str().is_none() {
                    return Err(diagnostic_at(source, fence_start, ParseErrorKind::InvalidToml));
                }
                properties.insert(key.clone(), value.to_string().trim_matches('"').to_string());
            }
            let theme_value_range =
                find_quoted_value_range(source, toml_block.body_range.clone(), "theme");
            if table.contains_key("theme") && theme_value_range.is_none() {
                return Err(diagnostic_at(source, fence_start, ParseErrorKind::InvalidToml));
            }
            let property_source = GlobalPropertySource {
                body_range: toml_block.body_range.clone(),
                theme_value_range,
            };
            return Ok(GlobalPropsResult { properties, property_source: Some(property_source) });
        } else if !line.trim().is_empty() {
            c.idx = save;
            return Ok(GlobalPropsResult { properties: HashMap::new(), property_source: None });
        } else {
            c.idx += 1;
        }
    }
    c.idx = save;
    Ok(GlobalPropsResult { properties: HashMap::new(), property_source: None })
}

struct OpenNode {
    level: u8,
    node: Node,
    note_lines: Vec<String>,
    props_done: bool,
}

fn parse_heading_tree(src: &str, c: &mut Cursor) -> Result<Node, MmfDiagnostic> {
    // 跳过前导空白行
    while c.idx < c.lines.len() && c.lines[c.idx].trim().is_empty() {
        c.idx += 1;
    }

    // 读第一个 heading
    let root_h = scan_next_heading(src, c)
        .ok_or_else(|| diagnostic_at(src, 0, ParseErrorKind::EmptyDocument))?;
    if root_h.level != ROOT_HEADING_LEVEL {
        return Err(diagnostic_for_heading(&root_h, ParseErrorKind::HeadingLevelSkip));
    }
    // 保留根节点之前的字节作为虚拟根的范围
    let root_start = 0usize;

    let mut stack: Vec<OpenNode> = vec![OpenNode {
        level: root_h.level,
        node: Node {
            title: root_h.title.clone(),
            children: vec![],
            props: None,
            note: None,
            source_range: root_start..root_h.source_end,
            subtree_source_range: root_start..root_h.source_end,
            title_byte_range: root_h.title_byte_range,
            heading_marker_range: root_h.heading_marker_range,
            child_insertion_byte: root_start,
            heading_level: root_h.level,
            property_source: None,
            heading_source_end: root_h.source_end,
        },
        note_lines: vec![],
        props_done: false,
    }];

    let mut active_fence = None;

    // 看当前行
    while c.idx < c.lines.len() {
        let line = c.lines[c.idx];

        if let Some(fence) = active_fence {
            append_note_line(&mut stack, line);
            if is_closing_fence(line, fence) {
                active_fence = None;
            }
            c.idx += 1;
            continue;
        }

        // ```toml node → 绑定到栈顶节点
        let top_accepts_props =
            !stack.last().expect("heading parser always retains the root node").props_done;
        if line.trim() == "```toml node" && top_accepts_props {
            let fence_start = line_start(src, line);
            c.idx += 1;
            let toml_block = collect_toml_block(src, c)
                .ok_or_else(|| diagnostic_at(src, fence_start, ParseErrorKind::InvalidToml))?;
            let (props, property_source) = parse_node_props(src, fence_start, &toml_block)?;
            let top = stack.last_mut().expect("heading parser always retains the root node");
            top.node.props = Some(props);
            top.node.property_source = Some(property_source);
            top.props_done = true;
            continue;
        }

        if let Some(fence) = fence_delimiter(line) {
            append_note_line(&mut stack, line);
            active_fence = Some(fence);
            c.idx += 1;
            continue;
        }

        // 遇到新 heading？
        if let Some(h) = peek_heading(src, line, c.idx + 1) {
            if h.level == ROOT_HEADING_LEVEL {
                return Err(diagnostic_for_heading(&h, ParseErrorKind::MultipleRoots));
            }
            let current_level =
                stack.last().expect("heading parser always retains the root node").level;
            if h.level > current_level + ROOT_HEADING_LEVEL {
                return Err(diagnostic_for_heading(&h, ParseErrorKind::HeadingLevelSkip));
            }
            close_nodes_for_heading(&mut stack, h.level, h.source_start);
            let new_node = Node {
                title: h.title.clone(),
                children: vec![],
                props: None,
                note: None,
                source_range: h.source_start..0, // 临时，结束时修正
                subtree_source_range: h.source_start..0,
                title_byte_range: h.title_byte_range,
                heading_marker_range: h.heading_marker_range,
                child_insertion_byte: h.source_start,
                heading_level: h.level,
                property_source: None,
                heading_source_end: h.source_end,
            };

            stack.push(OpenNode {
                level: h.level,
                node: new_node,
                note_lines: vec![],
                props_done: false,
            });
            c.idx += 1;
            continue;
        }

        // 普通行 → 追加到栈顶 note
        append_note_line(&mut stack, line);
        c.idx += 1;
    }

    // 文件结束 → 弹出所有节点，修正 source_range
    let end = src.len();
    let mut result: Option<Node> = None;
    while let Some(open) = stack.pop() {
        let mut n = open.node;
        let subtree_start = n.source_range.start;
        set_subtree_range(&mut n, subtree_start..end);
        n.note = if open.note_lines.is_empty() { None } else { Some(open.note_lines.join("\n")) };
        if let Some(parent) = stack.last_mut() {
            parent.node.children.push(n);
        } else {
            result = Some(n);
        }
    }
    let mut tree_root =
        result.ok_or_else(|| diagnostic_at(src, 0, ParseErrorKind::EmptyDocument))?;
    assign_child_insertion_bytes(&mut tree_root);
    Ok(tree_root)
}

// ── 辅助函数 ──

fn close_nodes_for_heading(stack: &mut Vec<OpenNode>, level: u8, source_end: usize) {
    // Always keep the root node (at index 0)
    while stack.len() > 1
        && stack.last().expect("stack length check guarantees a last node").level >= level
    {
        let closed = stack.pop().expect("stack length check guarantees a node to pop");
        let parent = stack.last_mut().expect("root node remains after closing a non-root node");
        finish_node_and_push(closed, source_end, &mut parent.node);
    }
}

fn finish_node_and_push(open: OpenNode, end: usize, parent: &mut Node) {
    let mut n = open.node;
    let subtree_start = n.source_range.start;
    set_subtree_range(&mut n, subtree_start..end);
    n.note = if open.note_lines.is_empty() { None } else { Some(open.note_lines.join("\n")) };
    parent.children.push(n);
}

struct HeadingInfo {
    title: String,
    level: u8,
    source_start: usize,
    source_end: usize,
    title_byte_range: Range<usize>,
    heading_marker_range: Range<usize>,
    line_number: usize,
}

fn peek_heading(src: &str, line: &str, line_number: usize) -> Option<HeadingInfo> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed.chars().take_while(|&c| c == '#').count() as u8;
    let after_hashes = &trimmed[level as usize..];
    let title = after_hashes.trim_start();

    let source_start = line_start(src, line);
    let source_end = source_start + line.len();
    let leading_space_count = line.len() - trimmed.len();
    let marker_start = source_start + leading_space_count;
    let marker_end = marker_start + level as usize;

    let title_start = title.as_ptr() as usize - src.as_ptr() as usize;
    let title_end = title_start + title.len();

    Some(HeadingInfo {
        title: title.to_string(),
        level,
        source_start,
        source_end,
        title_byte_range: title_start..title_end,
        heading_marker_range: marker_start..marker_end,
        line_number,
    })
}

fn scan_next_heading(src: &str, c: &mut Cursor) -> Option<HeadingInfo> {
    let mut active_fence = None;
    while c.idx < c.lines.len() {
        let line = c.lines[c.idx];
        if let Some(fence) = active_fence {
            if is_closing_fence(line, fence) {
                active_fence = None;
            }
            c.idx += 1;
            continue;
        }
        if let Some(fence) = fence_delimiter(line) {
            active_fence = Some(fence);
            c.idx += 1;
            continue;
        }
        if let Some(h) = peek_heading(src, line, c.idx + 1) {
            c.idx += 1;
            return Some(h);
        }
        c.idx += 1;
    }
    None
}

#[derive(Clone, Copy)]
struct Fence {
    marker: char,
    length: usize,
}

fn fence_delimiter(line: &str) -> Option<Fence> {
    let trimmed = line.trim_start();
    let marker = trimmed.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let length = trimmed.chars().take_while(|character| *character == marker).count();
    (length >= MINIMUM_FENCE_LENGTH).then_some(Fence { marker, length })
}

fn is_closing_fence(line: &str, opening: Fence) -> bool {
    let trimmed = line.trim_start();
    let length = trimmed.chars().take_while(|character| *character == opening.marker).count();
    length >= opening.length && trimmed[length..].trim().is_empty()
}

struct TomlBlock {
    source: String,
    body_range: Range<usize>,
}

fn collect_toml_block(source: &str, c: &mut Cursor) -> Option<TomlBlock> {
    let mut toml_source = String::new();
    let mut body_start = None;
    while c.idx < c.lines.len() {
        let line = c.lines[c.idx];
        if line.trim() == "```" {
            let body_end = line_start(source, line);
            c.idx += 1;
            return Some(TomlBlock {
                source: toml_source,
                body_range: body_start.unwrap_or(body_end)..body_end,
            });
        }
        body_start.get_or_insert_with(|| line_start(source, line));
        toml_source.push_str(line);
        toml_source.push('\n');
        c.idx += 1;
    }
    None
}

fn line_end(source: &str, start: usize, limit: usize) -> (usize, bool) {
    let Some(relative_end) = source[start..limit].find('\n') else {
        return (limit, false);
    };
    (start + relative_end, true)
}

pub(crate) fn parse_global_property_source(
    source: &str,
) -> Result<Option<GlobalPropertySource>, MmfDiagnostic> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() || source.trim().is_empty() {
        return Ok(None);
    }
    let mut cursor = Cursor { lines, idx: 0 };
    let result = parse_global_props(source, &mut cursor)?;
    Ok(result.property_source)
}

fn find_quoted_value_range(
    source: &str,
    body_range: Range<usize>,
    key: &str,
) -> Option<Range<usize>> {
    let mut line_start = body_range.start;
    while line_start < body_range.end {
        let (line_end, has_newline) = line_end(source, line_start, body_range.end);
        let line = &source[line_start..line_end];
        if let Some(equal_index) = line.find('=')
            && line[..equal_index].trim() == key
        {
            let value_source = &line[equal_index + 1..];
            let leading_whitespace = value_source.len() - value_source.trim_start().len();
            let value_start = line_start + equal_index + 1 + leading_whitespace;
            let value = &source[value_start..line_end];
            let quote = value.as_bytes().first().copied()?;
            let quote_delimiter = [quote; TRIPLE_QUOTE_LENGTH];
            let is_triple_quoted = value.as_bytes().get(..TRIPLE_QUOTE_LENGTH)
                == Some(quote_delimiter.as_slice())
                && (quote == b'"' || quote == b'\'');
            if is_triple_quoted {
                let delimiter = std::str::from_utf8(&quote_delimiter).ok()?;
                let closing_offset = value[TRIPLE_QUOTE_LENGTH..].find(delimiter)?;
                return Some(value_start..value_start + closing_offset + TRIPLE_QUOTE_LENGTH * 2);
            }
            if quote == b'"' || quote == b'\'' {
                let opening_quote_len = 1;
                let mut byte_offset = opening_quote_len;
                while byte_offset < value.len() {
                    let byte = value.as_bytes()[byte_offset];
                    if byte == quote {
                        let backslash_count = value[..byte_offset]
                            .bytes()
                            .rev()
                            .take_while(|&byte| byte == b'\\')
                            .count();
                        if backslash_count % 2 == 0 {
                            return Some(value_start..value_start + byte_offset + 1);
                        }
                    }
                    byte_offset += 1;
                }
            }
        }
        if !has_newline {
            break;
        }
        line_start = line_end + 1;
    }
    None
}

fn find_collapsed_value_range(
    source: &str,
    body_start: usize,
    body_end: usize,
) -> Option<Range<usize>> {
    let mut line_start = body_start;
    while line_start < body_end {
        let (line_end, has_newline) = line_end(source, line_start, body_end);
        let line = &source[line_start..line_end];
        if let Some(equal_index) = line.find('=')
            && line[..equal_index].trim() == "collapsed"
        {
            let value_source = &line[equal_index + 1..];
            let leading_whitespace = value_source.len() - value_source.trim_start().len();
            let value_start = line_start + equal_index + 1 + leading_whitespace;
            let value = &source[value_start..line_end];
            for literal in ["true", "false"] {
                if value.starts_with(literal)
                    && value[literal.len()..].chars().next().is_none_or(char::is_whitespace)
                {
                    return Some(value_start..value_start + literal.len());
                }
            }
        }
        if !has_newline {
            break;
        }
        line_start = line_end + 1;
    }
    None
}

fn parse_node_props(
    source: &str,
    fence_start: usize,
    toml_block: &TomlBlock,
) -> Result<(NodeProps, NodePropertySource), MmfDiagnostic> {
    let toml::Value::Table(table) = toml_block
        .source
        .parse::<toml::Value>()
        .map_err(|_| diagnostic_at(source, fence_start, ParseErrorKind::InvalidToml))?
    else {
        return Err(diagnostic_at(source, fence_start, ParseErrorKind::InvalidToml));
    };
    let props = NodeProps {
        id: table.get("id").and_then(|value| value.as_str().map(String::from)),
        priority: table.get("priority").and_then(|value| value.as_str().map(String::from)),
        status: table.get("status").and_then(|value| value.as_str().map(String::from)),
        owner: table.get("owner").and_then(|value| value.as_str().map(String::from)),
        collapsed: table.get("collapsed").and_then(toml::Value::as_bool).unwrap_or(false),
        tags: table
            .get("tags")
            .and_then(toml::Value::as_array)
            .map(|values| {
                values.iter().filter_map(|value| value.as_str().map(String::from)).collect()
            })
            .unwrap_or_default(),
        color: table.get("color").and_then(|value| value.as_str().map(String::from)),
    };
    let property_source = NodePropertySource {
        body_range: toml_block.body_range.clone(),
        collapsed_value_range: find_collapsed_value_range(
            source,
            toml_block.body_range.start,
            toml_block.body_range.end,
        ),
    };
    Ok((props, property_source))
}

fn append_note_line(stack: &mut [OpenNode], line: &str) {
    let node = stack.last_mut().expect("heading parser always retains the root node");
    if !line.trim().is_empty() || !node.note_lines.is_empty() {
        node.note_lines.push(line.to_string());
    }
}

fn set_subtree_range(node: &mut Node, source_range: Range<usize>) {
    node.source_range = source_range.clone();
    node.subtree_source_range = source_range;
}

fn assign_child_insertion_bytes(node: &mut Node) {
    node.child_insertion_byte = node
        .children
        .first()
        .map(|child| child.subtree_source_range.start)
        .unwrap_or(node.subtree_source_range.end);
    for child in &mut node.children {
        assign_child_insertion_bytes(child);
    }
}

fn diagnostic_for_heading(heading: &HeadingInfo, kind: ParseErrorKind) -> MmfDiagnostic {
    MmfDiagnostic::new(
        kind,
        heading.line_number,
        heading.heading_marker_range.start - heading.source_start + 1,
    )
}

fn diagnostic_at(source: &str, byte: usize, kind: ParseErrorKind) -> MmfDiagnostic {
    let bounded_byte = byte.min(source.len());
    let prefix = &source[..bounded_byte];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix.rfind('\n').map_or(bounded_byte + 1, |newline| bounded_byte - newline);
    MmfDiagnostic::new(kind, line, column)
}

fn line_start(source: &str, line: &str) -> usize {
    line.as_ptr() as usize - source.as_ptr() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_mmf() {
        let src = "# 产品规划\n\n## 数据同步\n\n## AI 生成\n";
        let tree = parse(src).expect("parse minimal mmf");
        assert_eq!(tree.root.title, "产品规划");
        assert_eq!(tree.root.heading_level, 1);
        assert_eq!(tree.root.children.len(), 2);
        assert_eq!(tree.root.children[0].title, "数据同步");
        assert_eq!(tree.root.children[0].heading_level, 2);
    }

    #[test]
    fn parse_node_with_props() {
        let src = "\
# Root

## Tasks

```toml node
priority = \"P1\"
status = \"todo\"
```

Some note here.

### SubTask
";
        let tree = parse(src).expect("parse with props");
        let tasks = &tree.root.children[0];
        assert_eq!(tasks.title, "Tasks");
        let props = tasks.props.as_ref().expect("should have props");
        assert_eq!(props.priority.as_deref(), Some("P1"));
        assert_eq!(props.status.as_deref(), Some("todo"));
        assert_eq!(tasks.note.as_deref(), Some("Some note here.\n"));
        assert_eq!(tasks.children.len(), 1);
        assert_eq!(tasks.children[0].title, "SubTask");
    }

    #[test]
    fn property_source_records_existing_collapsed_value() {
        let source = "# Root\n## Child\n```toml node\ncollapsed = false\npriority = \"P1\"\n```\n";
        let tree = parse(source).expect("fixture must parse");
        let child = &tree.root.children[0];
        let property_source = child.property_source.as_ref().expect("property source");
        let body_start = source.find("collapsed = false").expect("collapsed field");
        let body_end = source.find("```\n").expect("closing fence");
        let value_start = source.find("false").expect("boolean value");
        assert_eq!(property_source.body_range, body_start..body_end);
        assert_eq!(
            property_source.collapsed_value_range,
            Some(value_start..value_start + "false".len())
        );
    }

    #[test]
    fn node_retains_property_source_and_heading_end() {
        let source = "# Root\n## Child\n```toml node\ncollapsed = false\n```\n";
        let tree = parse(source).expect("fixture must parse");
        let child = &tree.root.children[0];
        let property_source = child.property_source.as_ref().expect("property source");
        let body_start = source.find("collapsed = false").expect("collapsed field");
        let body_end = source.find("```\n").expect("closing fence");
        assert_eq!(property_source.body_range, body_start..body_end);
        let child_heading_end = source.find("\n```toml node").expect("heading newline");
        assert_eq!(child.heading_source_end, child_heading_end);
    }

    #[test]
    fn property_source_does_not_cross_into_child_nodes() {
        let source = "# Root\n## Parent\n### Child\n```toml node\ncollapsed = true\n```\n";
        let tree = parse(source).expect("fixture must parse");
        let parent = &tree.root.children[0];
        assert!(parent.property_source.is_none());
        assert!(tree.root.children[0].children[0].property_source.is_some());
    }

    #[test]
    fn property_source_preserves_crlf_ranges() {
        let source = "# Root\r\n## Child\r\n```toml node\r\ncollapsed = false\r\n```\r\n";
        let tree = parse(source).expect("fixture must parse");
        let child = &tree.root.children[0];
        let property_source = child.property_source.as_ref().expect("property source");
        let body_start = source.find("collapsed = false").expect("collapsed field");
        let body_end = source.find("```\r\n").expect("closing fence");
        let value_start = source.find("false").expect("boolean value");
        assert_eq!(property_source.body_range, body_start..body_end);
        assert_eq!(
            property_source.collapsed_value_range,
            Some(value_start..value_start + "false".len())
        );
    }

    #[test]
    fn property_source_records_missing_collapsed_field() {
        let source = "# Root\n## Child\n```toml node\npriority = \"P1\"\n```\n";
        let tree = parse(source).expect("fixture must parse");
        let child = &tree.root.children[0];
        let property_source = child.property_source.as_ref().expect("property source");
        assert!(property_source.collapsed_value_range.is_none());
    }

    #[test]
    fn property_source_is_absent_without_a_property_block() {
        let source = "# Root\n## Child\n";
        let tree = parse(source).expect("fixture must parse");
        let child = &tree.root.children[0];
        assert!(child.property_source.is_none());
    }

    #[test]
    fn parse_global_props() {
        let src = "\
```toml mindmap
version = 1
layout = \"auto\"
```

# Root
";
        let tree = parse(src).expect("parse global props");
        assert_eq!(tree.version, 1);
        assert_eq!(tree.global_props.get("layout").map(|s| s.as_str()), Some("auto"));
    }

    #[test]
    fn global_property_source_records_theme_literal_range() {
        let source = "```toml mindmap\nversion = 1\ntheme = \"dawn\"\n```\n\n# Root\n";
        let property_source = parse_global_property_source(source)
            .expect("valid mmap metadata")
            .expect("global property source");
        let value_start = source.find("\"dawn\"").expect("theme literal");
        assert_eq!(property_source.theme_value_range, Some(value_start..value_start + 6));
        assert_eq!(&source[property_source.body_range], "version = 1\ntheme = \"dawn\"\n");
    }

    #[test]
    fn global_property_source_records_single_quoted_theme_literal_range() {
        let source = "```toml mindmap\nversion = 1\ntheme = 'dawn'\n```\n\n# Root\n";
        let property_source = parse_global_property_source(source)
            .expect("valid metadata")
            .expect("global property source");
        let range = property_source.theme_value_range.expect("theme range");
        assert_eq!(&source[range], "'dawn'");
    }

    #[test]
    fn global_property_source_records_single_line_triple_quoted_theme_literal_range() {
        let source = "```toml mindmap\nversion = 1\ntheme = \"\"\"dawn\"\"\"\n```\n\n# Root\n";
        let property_source = parse_global_property_source(source)
            .expect("valid metadata")
            .expect("global property source");
        let range = property_source.theme_value_range.expect("theme range");
        assert_eq!(&source[range], "\"\"\"dawn\"\"\"");
    }

    #[test]
    fn global_property_source_records_single_line_triple_literal_theme_range() {
        let source = "```toml mindmap\nversion = 1\ntheme = '''dawn'''\n```\n\n# Root\n";
        let property_source = parse_global_property_source(source)
            .expect("valid metadata")
            .expect("global property source");
        let range = property_source.theme_value_range.expect("theme range");
        assert_eq!(&source[range], "'''dawn'''");
    }

    #[test]
    fn multiline_theme_literal_without_a_replaceable_range_is_rejected() {
        let source = "```toml mindmap\nversion = 1\ntheme = \"\"\"\ndawn\n\"\"\"\n```\n\n# Root\n";
        let error = parse(source).expect_err("multiline theme metadata must be rejected");
        assert_eq!(error.kind, ParseErrorKind::InvalidToml);
    }

    #[test]
    fn global_property_source_preserves_crlf_ranges() {
        let source = "```toml mindmap\r\nversion = 1\r\ntheme = \"tide\"\r\n```\r\n# Root\r\n";
        let range = parse_global_property_source(source)
            .expect("valid CRLF mmap metadata")
            .expect("global property source")
            .theme_value_range
            .expect("theme range");
        assert_eq!(&source[range], "\"tide\"");
    }

    #[test]
    fn global_theme_requires_a_string_literal() {
        let error = parse("```toml mindmap\ntheme = 7\n```\n# Root\n")
            .expect_err("numeric theme metadata must be rejected");
        assert_eq!(error.kind, ParseErrorKind::InvalidToml);
    }

    #[test]
    fn title_byte_ranges_are_correct() {
        let src = "# 产品\n\n## 数据同步\n";
        let tree = parse(src).expect("parse");
        // "# 产品\n" → title starts after "# "
        assert_eq!(&src[tree.root.title_byte_range.clone()], "产品");
        // "## 数据同步\n" → title starts after "## "
        assert_eq!(&src[tree.root.children[0].title_byte_range.clone()], "数据同步");
    }

    #[test]
    fn empty_heading_keeps_zero_length_title_range() {
        let source = "# Root\n##\n";
        let child = &parse(source).expect("empty title is valid").root.children[0];
        assert_eq!(child.title, "");
        assert_eq!(child.title_byte_range.start, child.title_byte_range.end);
        assert_eq!(&source[child.heading_marker_range.clone()], "##");
    }

    #[test]
    fn fenced_hash_line_is_note_content_not_a_child() {
        let source = "# Root\n\n```text\n## not a node\n```\n\n## Child\n";
        let tree = parse(source).expect("parse fenced note");
        assert_eq!(tree.root.children.len(), 1);
        assert_eq!(tree.root.children[0].title, "Child");
    }

    #[test]
    fn node_ranges_distinguish_child_insertion_and_subtree_end() {
        let source =
            "# Root\nroot note\n## Parent\nparent note\n### Existing\nchild note\n## Next\n";
        let tree = parse(source).expect("parse ranges");
        let parent = &tree.root.children[0];
        assert_eq!(
            parent.child_insertion_byte,
            source.find("### Existing").expect("existing child heading")
        );
        assert_eq!(
            parent.subtree_source_range.end,
            source.find("## Next").expect("next sibling heading")
        );
        assert_eq!(&source[parent.heading_marker_range.clone()], "##");
    }

    #[test]
    fn leaf_with_note_inserts_children_at_subtree_end() {
        let source = "# Root\n## Leaf\nleaf note\n";
        let tree = parse(source).expect("parse leaf note");
        let leaf = &tree.root.children[0];
        assert!(leaf.children.is_empty());
        assert_eq!(leaf.child_insertion_byte, leaf.subtree_source_range.end);
        assert_eq!(leaf.child_insertion_byte, source.len());
    }

    #[test]
    fn empty_document_reports_stable_source_location() {
        let diagnostic = parse(" \n\t").expect_err("blank document is invalid");
        assert_eq!(diagnostic.kind, ParseErrorKind::EmptyDocument);
        assert_eq!((diagnostic.line, diagnostic.column), (1, 1));
        assert_eq!(diagnostic.message, "MMF document is empty.");
    }

    #[test]
    fn multiple_roots_reports_stable_source_location() {
        let diagnostic = parse("# Root\n# Another Root\n").expect_err("only one root is valid");
        assert_eq!(diagnostic.kind, ParseErrorKind::MultipleRoots);
        assert_eq!((diagnostic.line, diagnostic.column), (2, 1));
        assert_eq!(diagnostic.message, "MMF document must contain exactly one root heading.");
    }

    #[test]
    fn invalid_global_toml_reports_stable_source_location() {
        let diagnostic = parse("```toml mindmap\ninvalid = [\n```\n# Root\n")
            .expect_err("invalid TOML metadata is rejected");
        assert_eq!(diagnostic.kind, ParseErrorKind::InvalidToml);
        assert_eq!((diagnostic.line, diagnostic.column), (1, 1));
        assert_eq!(diagnostic.message, "MMF TOML metadata is invalid.");
    }

    #[test]
    fn non_level_one_root_reports_stable_source_location() {
        let diagnostic = parse("## NotRoot\n").expect_err("root must use one heading marker");
        assert_eq!(diagnostic.kind, ParseErrorKind::HeadingLevelSkip);
        assert_eq!((diagnostic.line, diagnostic.column), (1, 1));
        assert_eq!(diagnostic.message, "MMF heading levels must form a valid tree.");
    }
}
