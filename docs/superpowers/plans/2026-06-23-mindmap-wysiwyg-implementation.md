# Mindmap WYSIWYG 编辑器实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 基于 MMF v0.1 规范实现右分支树思维导图 ViewPlugin，支持画布点击定位光标、源码层文本编辑、InterceptKey 结构编辑。

**Architecture:** 新建 `crates/markdown/src/mmf/` 模块（model/parser/layout/canvas/edit），由 `MindmapView` 协调。文本编辑走 app 层 DocumentView，光标/IME 物理坐标通过 `CursorRect` 查询桥接，结构编辑通过 `InterceptKey` + `doc.replace_range()` 精确局部修改。

**Tech Stack:** Rust, existing `ViewPlugin` trait, `DrawList` rendering, `Shaper` text measurement, `Theme` color system

## Global Constraints

- `.unwrap()` 必须替换为 `.expect("详细理由")`
- 禁止硬编码颜色/尺寸值，使用 Theme 或命名常量
- `MindmapEdit` 枚举绝不暴露到 `crates/ui/src/plugin.rs`
- 结构编辑用 `doc.replace_range()` 做精确局部替换，绝不序列化 Tree 覆盖源码
- 每次提交前确保 `cargo check` 通过

---

### Task 1: 扩展 PluginMessage / PluginQuery / PluginResponse

**Files:**
- Modify: `crates/ui/src/plugin.rs:19-96`

**Interfaces:**
- Produces: `PluginMessage::InterceptKey { key: KeyCode, modifiers: Modifiers }`
- Produces: `PluginQuery::HitTestCanvas { x: f32, y: f32, offset_x: f32, offset_y: f32 }`
- Produces: `PluginQuery::CursorRect(usize)`
- Produces: `PluginQuery::VisualMove { from_byte: usize, direction: Direction }`
- Produces: `PluginResponse::HitResult(Option<HitResult>)`
- Produces: `PluginResponse::CursorRect(Option<(f32, f32, f32)>)`
- Produces: `pub struct HitResult { pub byte_offset: usize, pub node_idx: usize }`
- Produces: `pub enum Direction { Up, Down, Left, Right }`

- [ ] **Step 1: 新增 PluginMessage::InterceptKey 变体**

在 `crates/ui/src/plugin.rs` 第 50 行 `ClearSelection,` 之后插入：

```rust
    /// 请求插件拦截处理结构编辑按键。
    /// 插件可调用 doc 的方法直接修改源码。返回 true 表示已消费。
    InterceptKey { key: crate::core::widget::KeyCode, modifiers: crate::core::widget::Modifiers },
```

> 注意：`PluginMessage` 枚举中 `PluginQuery` 也引用 `KeyCode`，但走不同的 import 路径。这里用 full path 避免额外 import。
> 实际更简洁的做法是在文件顶部 `use crate::core::widget::{KeyCode, Modifiers};` 然后写 `InterceptKey { key: KeyCode, modifiers: Modifiers }`。

- [ ] **Step 2: 新增 PluginQuery 变体**

在第 91 行 `ScrollAnchor,` 之后插入：

```rust
    /// 画布坐标 → 源码字节偏移。用于点击画布节点定位光标。
    HitTestCanvas { x: f32, y: f32, offset_x: f32, offset_y: f32 },
    /// 源码字节偏移 → 画布像素坐标 (x, y, height)。用于光标绘制和 IME 定位。
    CursorRect(usize),
    /// 从当前源码位置执行画布导航。
    VisualMove { from_byte: usize, direction: Direction },
```

- [ ] **Step 3: 新增 PluginResponse 变体**

在第 111 行 `ScrollAnchor(String, f32),` 之后插入：

```rust
    HitResult(Option<HitResult>),
    /// 光标的画布物理坐标 (x, y, height)。None = 字节偏移不在任何节点内。
    CursorRect(Option<(f32, f32, f32)>),
```

- [ ] **Step 4: 新增 HitResult 和 Direction 类型**

在 `crates/ui/src/plugin.rs` 中，`FlatLine` 定义之后（约第 131 行）插入：

```rust
/// Hit-test 结果：画布坐标 → 源码信息。
#[derive(Debug, Clone)]
pub struct HitResult {
    pub byte_offset: usize,  // 在源码中的精确字节位置
    pub node_idx: usize,     // 命中的节点 DFS 索引
}

/// 画布方向导航。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction { Up, Down, Left, Right }
```

- [ ] **Step 5: 修复所有 match 臂**

`cargo check -p ui 2>&1` 查看编译错误，在以下位置补充新变体的 match 臂：
- `PluginMessage` 的 `InterceptKey` → 任何 match `PluginMessage` 的地方（如 `MarkdownView::handle_message`、app dispatch 层），添加 `InterceptKey { .. } => false`
- `PluginQuery` 的三个新变体 → 任何 match `PluginQuery` 的地方，返回 `PluginResponse::None`
- `PluginResponse` 的两个新变体 → 任何 match `PluginResponse` 的地方，添加 `HitResult(_) | CursorRect(_) => {}` 或类似处理

通过 `cargo check 2>&1 | head -30` 确认修复全部编译错误。

- [ ] **Step 6: Commit**

```bash
git add crates/ui/src/plugin.rs
# 以及其他被修改的文件（match 臂修复）
git commit -m "feat(ui): 新增 InterceptKey/CursorRect/VisualMove 等 PluginMessage/Query 变体

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 2: 扩展 DocViewMut——支持 replace_range

**Files:**
- Modify: `crates/core/src/document.rs:128-131`

**Interfaces:**
- Produces: `DocViewMut::replace_range(&mut self, range: Range<usize>, text: &str)`
- Produces: `DocViewMut::begin_edit(&mut self)` (default no-op)
- Produces: `DocViewMut::end_edit(&mut self)` (default no-op)

- [ ] **Step 1: 在 DocViewMut trait 中新增方法**

将 `crates/core/src/document.rs` 第 128-131 行的 trait 改为：

```rust
pub trait DocViewMut: DocView {
    fn set_scroll_y(&mut self, y: f32);

    /// 替换 [range] 字节区间的文本为 text。
    fn replace_range(&mut self, range: std::ops::Range<usize>, text: &str);

    /// 开始一个编辑事务——后续多次 replace_range 合并为一个 Undo 单元。
    fn begin_edit(&mut self) {}

    /// 结束编辑事务。
    fn end_edit(&mut self) {}
}
```

- [ ] **Step 2: 在 DocumentView 上实现新方法**

在 `crates/app/src/document_view/mod.rs` 的 `impl DocViewMut for DocumentView` 块中（或新建 `edit_trait.rs`），添加：

```rust
fn replace_range(&mut self, range: std::ops::Range<usize>, text: &str) {
    self.tb.replace_range(range, text);
    // replace_range 之后需要重建 line_index
    self.rebuild_viewport();
    self.dirty = true;
}

fn begin_edit(&mut self) {
    self.tb.edit_begin_grouping();
}

fn end_edit(&mut self) {
    self.tb.edit_end_grouping();
    self.rebuild_viewport();
    self.dirty = true;
}
```

> 注意：`TextBuffer::replace_range` 在 `crates/core/src/buffer/edit.rs:779`。
> `edit_begin_grouping()` / `edit_end_grouping()` 是公开 API（非 `edit_begin()` / `edit_end()` 私有方法）。

- [ ] **Step 3: cargo check 验证**

```bash
cargo check 2>&1
```

确保 `DocViewMut` 的所有实现者都实现了 `replace_range`。搜索：

```bash
grep -rn "impl DocViewMut" crates/
```

为每个实现者添加 `replace_range`（测试 mock 中可留空 `unimplemented!()`）。

- [ ] **Step 4: Commit**

```bash
git add crates/core/src/document.rs crates/app/src/document_view/
git commit -m "feat(core): DocViewMut 新增 replace_range/begin_edit/end_edit

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 3: MMF 数据模型 + 解析器

**Files:**
- Create: `crates/markdown/src/mmf/mod.rs`
- Create: `crates/markdown/src/mmf/model.rs`
- Create: `crates/markdown/src/mmf/parser.rs`
- Test: `crates/markdown/tests/mmf_parser_test.rs`（或内联 `#[cfg(test)]` 在 parser.rs 底部）

**Interfaces:**
- Produces: `mmf::Tree`, `mmf::Node`, `mmf::NodeProps` (pub structs)
- Produces: `mmf::parser::parse(source: &str) -> Result<Tree, ParseError>`

- [ ] **Step 1: 编写解析器测试（先写失败的测试）**

创建 `crates/markdown/src/mmf/parser.rs`，底部 `#[cfg(test)]` 块：

```rust
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
    fn title_byte_ranges_are_correct() {
        let src = "# 产品\n\n## 数据同步\n";
        let tree = parse(src).expect("parse");
        // "# 产品\n" → title starts after "# "
        assert_eq!(&src[tree.root.title_byte_range.clone()], "产品");
        // "## 数据同步\n" → title starts after "## "
        assert_eq!(&src[tree.root.children[0].title_byte_range.clone()], "数据同步");
    }
}
```

运行测试确认失败：

```bash
cargo test -p edit-plus-markdown mmf_parser -- 2>&1 | tail -5
# Expected: FAIL - module not found
```

- [ ] **Step 2: 添加 toml 依赖**

在 `crates/markdown/Cargo.toml` 的 `[dependencies]` 中添加：

```toml
toml.workspace = true
```

- [ ] **Step 3: 定义数据模型**

创建 `crates/markdown/src/mmf/model.rs`：

```rust
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
    /// 标题文字在源码中的字节范围（不含 `# ` 前缀和换行符）
    pub title_byte_range: Range<usize>,
    /// `#` 的个数：1=根, 2=一级子, ...
    pub heading_level: u8,
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

#[derive(Debug)]
pub enum ParseError {
    EmptyDocument,
    MultipleRoots,
    InvalidToml(String),
    HeadingLevelSkip { line: usize },
}
```

创建 `crates/markdown/src/mmf/mod.rs`：

```rust
pub mod model;
pub mod parser;

pub use model::*;
```

在 `crates/markdown/src/lib.rs` 中添加 `pub mod mmf;`。

- [ ] **Step 4: 实现解析器（栈式算法）**

在 `crates/markdown/src/mmf/parser.rs` 中：

```rust
use std::collections::HashMap;
use std::ops::Range;
use super::model::*;

pub fn parse(source: &str) -> Result<Tree, ParseError> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() || source.trim().is_empty() {
        return Err(ParseError::EmptyDocument);
    }
    let mut cursor = Cursor { lines, idx: 0 };

    // 1. 全局属性
    let global_props = parse_global_props(source, &mut cursor);

    // 2. 栈式解析 heading 树
    let root = parse_heading_tree(source, &mut cursor)?;
    Ok(Tree { version: 1, root, global_props })
}

struct Cursor<'a> {
    lines: Vec<&'a str>,
    idx: usize,
}

fn line_byte_range(src: &str, target_line: &str, search_from: usize) -> Range<usize> {
    let pos = src[search_from..]
        .find(target_line)
        .map(|p| search_from + p)
        .expect("line not found in source");
    let len = target_line.len();
    pos..(pos + len)
}

fn parse_global_props(src: &str, c: &mut Cursor) -> HashMap<String, String> {
    let save = c.idx;
    while c.idx < c.lines.len() {
        let line = c.lines[c.idx];
        if line.trim() == "```toml mindmap" {
            c.idx += 1;
            let mut toml_str = String::new();
            while c.idx < c.lines.len() {
                let inner = c.lines[c.idx];
                if inner.trim() == "```" { c.idx += 1; break; }
                toml_str.push_str(inner);
                toml_str.push('\n');
                c.idx += 1;
            }
            if let Ok(table) = toml_str.parse::<toml::Table>() {
                let mut m = HashMap::new();
                for (k, v) in &table {
                    m.insert(k.clone(), v.to_string().trim_matches('"').to_string());
                }
                return m;
            }
        } else if !line.trim().is_empty() {
            c.idx = save;
            return HashMap::new();
        } else {
            c.idx += 1;
        }
    }
    c.idx = save;
    HashMap::new()
}

struct OpenNode {
    level: u8,
    node: Node,
    note_lines: Vec<String>,
    props_done: bool,
}

fn parse_heading_tree(src: &str, c: &mut Cursor) -> Result<Node, ParseError> {
    // 跳过前导空白行
    while c.idx < c.lines.len() && c.lines[c.idx].trim().is_empty() {
        c.idx += 1;
    }

    // 读第一个 heading
    let first = scan_next_heading(src, c);
    let root_h = first.expect("expected root heading");
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
            title_byte_range: root_h.title_byte_range,
            heading_level: root_h.level,
        },
        note_lines: vec![],
        props_done: false,
    }];

    loop {
        // 看当前行
        while c.idx < c.lines.len() {
            let line = c.lines[c.idx];

            // ```toml node → 绑定到栈顶节点
            if line.trim() == "```toml node" && !stack.is_empty() && !stack.last().unwrap().props_done {
                c.idx += 1;
                let mut toml_str = String::new();
                while c.idx < c.lines.len() {
                    let inner = c.lines[c.idx];
                    if inner.trim() == "```" { c.idx += 1; break; }
                    toml_str.push_str(inner);
                    toml_str.push('\n');
                    c.idx += 1;
                }
                if let Ok(t) = toml_str.parse::<toml::Table>() {
                    let top = stack.last_mut().unwrap();
                    top.node.props = Some(NodeProps {
                        id: t.get("id").and_then(|v| v.as_str().map(String::from)),
                        priority: t.get("priority").and_then(|v| v.as_str().map(String::from)),
                        status: t.get("status").and_then(|v| v.as_str().map(String::from)),
                        owner: t.get("owner").and_then(|v| v.as_str().map(String::from)),
                        collapsed: t.get("collapsed").and_then(|v| v.as_bool()).unwrap_or(false),
                        tags: t.get("tags").and_then(|v| v.as_array())
                            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                            .unwrap_or_default(),
                        color: t.get("color").and_then(|v| v.as_str().map(String::from)),
                    });
                    top.props_done = true;
                }
                continue;
            }

            // 遇到新 heading？
            if let Some(h) = peek_heading(line) {
                let popped = pop_to_level(&mut stack, h.level);
                let parent = stack.last_mut().expect("stack empty after pop");
                let mut new_node = Node {
                    title: h.title.clone(),
                    children: vec![],
                    props: None,
                    note: None,
                    source_range: h.source_start..0, // 临时，结束时修正
                    title_byte_range: h.title_byte_range,
                    heading_level: h.level,
                };

                // 计算刚关闭的节点的 source_range
                for closed in popped {
                    let end = h.source_start; // 新 heading 开始处即旧节点结束处
                    finish_node_and_push(closed, end, parent);
                }

                parent.children.push(new_node);
                // 重新获取 parent（上面 push 后 vec 可能移动）
                let child = parent.children.last_mut().unwrap();
                stack.push(OpenNode {
                    level: h.level,
                    node: Node { ..std::mem::take(child) },
                    note_lines: vec![],
                    props_done: false,
                });
                // 把 child 内容移到了 stack 中，需要清理
                let moved = stack.last_mut().unwrap();
                // 重新设置新节点的 source_start
                moved.node.source_range = h.source_start..0;
                c.idx += 1;
                continue;
            }

            // 普通行 → 追加到栈顶 note
            if !stack.is_empty() {
                if !line.trim().is_empty() || !stack.last().unwrap().note_lines.is_empty() {
                    stack.last_mut().unwrap().note_lines.push(line.to_string());
                }
            }
            c.idx += 1;
        }

        // 文件结束 → 弹出所有节点，修正 source_range
        let end = src.len();
        let mut result: Option<Node> = None;
        while let Some(open) = stack.pop() {
            let mut n = open.node;
            n.source_range = n.source_range.start..end;
            n.note = if open.note_lines.is_empty() {
                None
            } else {
                Some(open.note_lines.join("\n"))
            };
            if let Some(parent) = stack.last_mut() {
                parent.node.children.push(n);
            } else {
                result = Some(n);
            }
        }
        return result.ok_or(ParseError::EmptyDocument);
    }
}

// ── 辅助函数 ──

fn pop_to_level(stack: &mut Vec<OpenNode>, level: u8) -> Vec<OpenNode> {
    let mut popped = vec![];
    while stack.last().is_some_and(|n| n.level >= level) {
        popped.push(stack.pop().unwrap());
    }
    popped.reverse();
    popped
}

fn finish_node_and_push(open: OpenNode, end: usize, parent: &mut Node) {
    let mut n = open.node;
    n.source_range = n.source_range.start..end;
    n.note = if open.note_lines.is_empty() {
        None
    } else {
        Some(open.note_lines.join("\n"))
    };
    parent.children.push(n);
}

struct HeadingInfo {
    title: String,
    level: u8,
    source_start: usize,
    source_end: usize,
    title_byte_range: Range<usize>,
}

fn peek_heading(line: &str) -> Option<HeadingInfo> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') { return None; }
    let level = trimmed.chars().take_while(|&c| c == '#').count() as u8;
    let after_hashes = &trimmed[level as usize..];
    let title = after_hashes.trim_start().to_string();
    Some(HeadingInfo {
        title: title.clone(),
        level,
        source_start: 0,   // caller 负责计算
        source_end: 0,
        title_byte_range: 0..0, // caller 负责计算
    })
}

fn scan_next_heading(src: &str, c: &mut Cursor) -> Option<HeadingInfo> {
    while c.idx < c.lines.len() {
        let line = c.lines[c.idx];
        if let Some(mut h) = peek_heading(line) {
            let r = line_byte_range(src, line, 0);
            h.source_start = r.start;
            h.source_end = r.end;
            // title_byte_range: find title text after "## "
            let hash_end = r.start + line.chars()
                .take_while(|&c| c == '#' || c == ' ')
                .count();
            let title_len = h.title.len();
            h.title_byte_range = hash_end..(hash_end + title_len);
            c.idx += 1;
            return Some(h);
        }
        c.idx += 1;
    }
    None
}
```

- [ ] **Step 5: 运行测试确认通过**

```bash
cargo test -p edit-plus-markdown mmf_parser -- --nocapture 2>&1
```

修复任何解析 bug。确认三个测试全部 PASS。

- [ ] **Step 6: Commit**

```bash
git add crates/markdown/src/mmf/ crates/markdown/src/lib.rs crates/markdown/Cargo.toml
git commit -m "feat(mmf): 实现 MMF 数据模型和解析器 Tree←Node←NodeProps

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 4: 右分支树布局算法

**Files:**
- Create: `crates/markdown/src/mmf/layout.rs`
- Test: `crates/markdown/tests/mmf_layout_test.rs`（或内联 `#[cfg(test)]`）

**Interfaces:**
- Consumes: `mmf::Tree` (from Task 3)
- Consumes: `shaping::Shaper` (for text width measurement)
- Produces: `pub fn compute_layout(tree: &Tree, shaper: &mut Shaper, constants: &LayoutConstants) -> LayoutTree`
- Produces: `pub fn build_hit_map(tree: &Tree, layout: &LayoutTree, shaper: &mut Shaper, constants: &LayoutConstants) -> HitMap`

- [ ] **Step 1: 定义布局常量和输出类型**

在 `crates/markdown/src/mmf/layout.rs` 中：

```rust
use std::ops::Range;
use shaping::Shaper;
use ui::core::geom::Rect;
use super::model::*;

/// 布局常量（从 Theme 或默认值读入，非硬编码）
pub struct LayoutConstants {
    pub card_height: f32,        // 默认 32.0
    pub card_padding_x: f32,     // 默认 16.0
    pub card_padding_y: f32,     // 默认 6.0
    pub level_indent: f32,       // 默认 240.0
    pub sibling_gap: f32,        // 默认 8.0
    pub card_radius: f32,        // 默认 6.0
    pub connector_width: f32,    // 默认 1.5
}

impl Default for LayoutConstants {
    fn default() -> Self {
        Self {
            card_height: 32.0,
            card_padding_x: 16.0,
            card_padding_y: 6.0,
            level_indent: 240.0,
            sibling_gap: 8.0,
            card_radius: 6.0,
            connector_width: 1.5,
        }
    }
}

/// 单个节点的布局结果
#[derive(Debug, Clone)]
pub struct LayoutNode {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub node_idx: usize,           // DFS 序遍历索引
    pub depth: u8,
    pub connector_from: (f32, f32), // 连线起点（父右边缘中点）
    pub connector_to: (f32, f32),   // 连线终点（本节点左边缘中点）
}

/// 完整布局结果
pub struct LayoutTree {
    pub nodes: Vec<LayoutNode>,     // DFS 序
    pub total_w: f32,
    pub total_h: f32,
}
```

- [ ] **Step 2: 写测试**

```rust
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
        let lt = compute_layout(&tree, shaper, &constants);
        assert!((lt.nodes[0].x - 0.0).abs() < 1.0, "root x should be 0");
    }

    #[test]
    fn child_is_indented_right() {
        let tree = dummy_tree();
        let constants = LayoutConstants::default();
        let shaper = &mut Shaper::new().expect("shaper");
        let lt = compute_layout(&tree, shaper, &constants);
        let child = &lt.nodes[1]; // "Child1"
        assert!(child.x > lt.nodes[0].x + 50.0,
            "child should be to the right of root, child.x={}, root.x={}",
            child.x, lt.nodes[0].x);
    }

    #[test]
    fn siblings_are_stacked_vertically() {
        let tree = dummy_tree();
        let constants = LayoutConstants::default();
        let shaper = &mut Shaper::new().expect("shaper");
        let lt = compute_layout(&tree, shaper, &constants);
        // Child1 在 Child2 上面
        let child1_y = lt.nodes[1].y;
        let child2_y = lt.nodes[2].y;
        assert!(child2_y > child1_y + 10.0,
            "siblings should be stacked, child1.y={}, child2.y={}",
            child1_y, child2_y);
    }
}
```

运行确认失败：

```bash
cargo test -p edit-plus-markdown mmf_layout -- 2>&1 | tail -5
```

- [ ] **Step 3: 实现布局算法**

```rust
/// 字底向上计算子树高度
fn subtree_height(node: &Node, constants: &LayoutConstants) -> f32 {
    if node.children.is_empty() {
        return constants.card_height;
    }
    let children_h: f32 = node.children.iter()
        .map(|c| subtree_height(c, constants))
        .sum::<f32>()
        + (node.children.len() - 1) as f32 * constants.sibling_gap;
    children_h.max(constants.card_height)
}

/// 自顶向下分配坐标
fn assign_positions(
    node: &Node,
    depth: u8,
    y_offset: f32,
    node_idx: &mut usize,
    parent_connector_from: Option<(f32, f32)>,
    constants: &LayoutConstants,
    shaper: &mut Shaper,
    out: &mut Vec<LayoutNode>,
) {
    let x = depth as f32 * constants.level_indent;
    let title_w = measure_text(&node.title, shaper);
    let card_w = title_w + 2.0 * constants.card_padding_x;
    let sub_h = subtree_height(node, constants);
    let card_y = y_offset + (sub_h - constants.card_height) / 2.0;

    let idx = *node_idx;
    *node_idx += 1;

    // 计算连线端点
    let connector_to = (x, card_y + constants.card_height / 2.0); // 左边缘中点
    let connector_from = parent_connector_from.unwrap_or(connector_to);

    out.push(LayoutNode {
        x,
        y: card_y,
        w: card_w.max(60.0), // 最小宽度
        h: constants.card_height,
        node_idx: idx,
        depth,
        connector_from,
        connector_to,
    });

    // 分配子节点
    let this_connector = (x + card_w.max(60.0), card_y + constants.card_height / 2.0);
    let mut cursor = y_offset;
    for child in &node.children {
        let child_h = subtree_height(child, constants);
        assign_positions(
            child, depth + 1, cursor,
            node_idx, Some(this_connector),
            constants, shaper, out,
        );
        cursor += child_h + constants.sibling_gap;
    }
}

fn measure_text(text: &str, shaper: &mut Shaper) -> f32 {
    if text.is_empty() { return 0.0; }
    match shaper.shape(text) {
        Ok(run) => run.advance,
        Err(_) => text.len() as f32 * shaper.font_size() * 0.5, // fallback
    }
}

pub fn compute_layout(
    tree: &Tree,
    shaper: &mut Shaper,
    constants: &LayoutConstants,
) -> LayoutTree {
    let mut nodes = Vec::new();
    let mut node_idx = 0;
    assign_positions(
        &tree.root, 0, 0.0,
        &mut node_idx, None,
        constants, shaper, &mut nodes,
    );
    let total_h = subtree_height(&tree.root, constants);
    let total_w = nodes.iter().map(|n| n.x + n.w).max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap_or(0.0);
    LayoutTree { nodes, total_w, total_h }
}
```

- [ ] **Step 4: 实现 build_hit_map**

```rust
/// 每个节点的命中矩形和字符边界
pub struct HitMap {
    /// node_rects[i] = 节点 DFS 索引 i 的卡片矩形
    pub node_rects: Vec<Rect>,
    /// title_char_edges[i][j] = 节点 i 第 j 个字符的右边缘 x（卡片本地坐标）
    pub title_char_edges: Vec<Vec<f32>>,
}

pub fn build_hit_map(
    tree: &Tree,
    layout: &LayoutTree,
    shaper: &mut Shaper,
    constants: &LayoutConstants,
) -> HitMap {
    let n = layout.nodes.len();
    let mut node_rects = Vec::with_capacity(n);
    let mut title_char_edges = Vec::with_capacity(n);

    // DFS 收集节点引用，与 layout.nodes 对应
    let nodes = collect_nodes_dfs(&tree.root);

    for (i, ln) in layout.nodes.iter().enumerate() {
        node_rects.push(Rect::new(ln.x, ln.y, ln.w, ln.h));

        let node = &nodes[i];
        let text_x = ln.x + constants.card_padding_x;
        let edges: Vec<f32> = if node.title.is_empty() {
            vec![text_x]
        } else if let Ok(run) = shaper.shape(&node.title) {
            let mut x = text_x;
            let mut positions = Vec::with_capacity(node.title.chars().count() + 1);
            for cluster in &run.clusters {
                x += cluster.advance;
                positions.push(x);
            }
            positions
        } else {
            // fallback: 每个字符估计宽度
            let est = shaper.font_size() * 0.5;
            (0..=node.title.chars().count())
                .map(|i| text_x + i as f32 * est)
                .collect()
        };
        title_char_edges.push(edges);
    }

    HitMap { node_rects, title_char_edges }
}

fn collect_nodes_dfs(node: &Node) -> Vec<&Node> {
    let mut v = vec![node];
    for child in &node.children {
        v.extend(collect_nodes_dfs(child));
    }
    v
}
```

- [ ] **Step 5: 运行测试确认通过**

```bash
cargo test -p edit-plus-markdown mmf_layout -- --nocapture 2>&1
```

- [ ] **Step 6: 更新 mmf/mod.rs**

```rust
pub mod layout;
```

- [ ] **Step 7: Commit**

```bash
git add crates/markdown/src/mmf/
git commit -m "feat(mmf): 右分支树布局算法——Tree→LayoutTree + HitMap

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 5: 画布渲染

**Files:**
- Create: `crates/markdown/src/mmf/canvas.rs`

**Interfaces:**
- Consumes: `LayoutTree`, `HitMap` (from Task 4)
- Consumes: `ui::core::paint::DrawList`, `ui::core::geom::Rect`
- Consumes: `ui::Theme`
- Produces: `pub fn render(dl: &mut DrawList, layout: &LayoutTree, viewport: Rect, theme: &Theme, constants: &LayoutConstants, ...)`

- [ ] **Step 1: 实现视口裁剪函数**

```rust
use ui::core::geom::Rect;
use ui::core::paint::DrawList;
use ui::theme::Theme;
use super::layout::{LayoutTree, LayoutConstants, LayoutNode};

/// 返回可见节点索引范围 (start, end)
pub fn visible_range(layout: &LayoutTree, viewport: Rect, buffer: f32) -> (usize, usize) {
    let y_min = viewport.y - buffer;
    let y_max = viewport.bottom() + buffer;
    let nodes = &layout.nodes;
    // layout.nodes 按 y 有序，二分查找
    let start = nodes.partition_point(|n| n.y + n.h < y_min);
    let end = nodes.partition_point(|n| n.y < y_max);
    (start.min(nodes.len()), end.min(nodes.len()))
}
```

- [ ] **Step 2: 实现卡片渲染**

```rust
/// 渲染可见节点卡片和连线
pub fn render_cards_and_connectors(
    dl: &mut DrawList,
    layout: &LayoutTree,
    visible: (usize, usize),
    theme: &Theme,
    constants: &LayoutConstants,
) {
    let node_bg = theme.scope_color("mindmap.node_bg");
    let node_border = theme.scope_color("mindmap.node_border");
    let root_bg = theme.scope_color("mindmap.root_bg");
    let root_border = theme.scope_color("mindmap.root_border");
    let connector_color = theme.scope_color("mindmap.connector");

    for i in visible.0..visible.1 {
        let ln = &layout.nodes[i];
        let rect = Rect::new(ln.x, ln.y, ln.w, ln.h);

        // 连线（除根节点外）
        if ln.depth > 0 {
            draw_connector(dl, ln, connector_color, constants.connector_width);
        }

        // 卡片背景 + 边框
        let (bg, border) = if ln.depth == 0 {
            (root_bg, root_border)
        } else {
            (node_bg, node_border)
        };

        dl.fill_rounded(rect, bg, constants.card_radius);
        dl.stroke_rounded(rect, border, 1.0, constants.card_radius);
    }
}

fn draw_connector(dl: &mut DrawList, ln: &LayoutNode, color: [f32; 4], width: f32) {
    let (fx, fy) = ln.connector_from;
    let (tx, ty) = ln.connector_to;
    let mid_x = (fx + tx) / 2.0;

    // 直角折线：父右边缘 → 水平到中点 → 垂直到子左边缘
    // 水平段 (fx → mid_x, fy)
    dl.fill(
        Rect::new(fx, fy - width / 2.0, mid_x - fx, width),
        color,
    );
    // 垂直段 (mid_x, min(fy, ty) → max(fy, ty))
    let y_top = fy.min(ty);
    let y_h = (fy - ty).abs();
    dl.fill(
        Rect::new(mid_x - width / 2.0, y_top, width, y_h),
        color,
    );
    // 水平段 (mid_x → tx, ty)
    dl.fill(
        Rect::new(mid_x, ty - width / 2.0, tx - mid_x, width),
        color,
    );
}
```

- [ ] **Step 3: 实现文字渲染**

```rust
pub fn render_text(
    dl: &mut DrawList,
    layout: &LayoutTree,
    visible: (usize, usize),
    theme: &Theme,
    constants: &LayoutConstants,
    shaper: &mut shaping::Shaper,
    node_titles: &[&str],
) {
    let text_color = theme.scope_color("mindmap.text");
    let root_text_color = theme.scope_color("mindmap.root_text");

    for i in visible.0..visible.1 {
        let ln = &layout.nodes[i];
        let title = node_titles.get(i).copied().unwrap_or("");
        let color = if ln.depth == 0 { root_text_color } else { text_color };
        let text_x = ln.x + constants.card_padding_x;
        let baseline_y = ln.y + constants.card_padding_y + shaper.font_size();

        if !title.is_empty() {
            dl.text_shaped(text_x, baseline_y, shaper.font_size(), color, title, shaper);
        }
    }
}
```

- [ ] **Step 4: 组装 render() 入口**

```rust
pub fn render(
    dl: &mut DrawList,
    layout: &LayoutTree,
    viewport: Rect,
    theme: &Theme,
    constants: &LayoutConstants,
    shaper: &mut shaping::Shaper,
    node_titles: &[&str],
) {
    let visible = visible_range(layout, viewport, constants.card_height * 2.0);
    render_cards_and_connectors(dl, layout, visible, theme, constants);
    render_text(dl, layout, visible, theme, constants, shaper, node_titles);
}
```

- [ ] **Step 5: 更新 mmf/mod.rs**

```rust
pub mod canvas;
```

- [ ] **Step 6: Commit**

```bash
git add crates/markdown/src/mmf/
git commit -m "feat(mmf): 画布渲染——卡片/连线/文字 + 视口裁剪

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 6: 结构编辑——InterceptKey → 源码修改

**Files:**
- Create: `crates/markdown/src/mmf/edit.rs`

**Interfaces:**
- Consumes: `Tree` (from Task 3), `DocViewMut` (from Task 2)
- Consumes: `PluginMessage::InterceptKey { key: KeyCode, modifiers: Modifiers }` (from Task 1)
- Produces: `pub fn handle_intercept_key(key: KeyCode, modifiers: Modifiers, tree: &Tree, focus_byte: usize, doc: &mut dyn DocViewMut) -> bool`

- [ ] **Step 1: 定义内部 MindmapEdit 枚举和辅助函数**

```rust
use ui::core::widget::{KeyCode, Modifiers};
use core::document::{DocViewMut, DocView};
use crate::mmf::model::{Tree, Node};

/// 内部编辑命令——绝不暴露到 ui::plugin
enum MindmapEdit {
    Indent { node_idx: usize },
    Outdent { node_idx: usize },
    NewSibling { after_idx: usize, level: u8 },
    NewChild { parent_idx: usize, parent_level: u8 },
    Delete { node_idx: usize },
    MoveUp { node_idx: usize },
    MoveDown { node_idx: usize },
}

/// 通过字节偏移找到所属节点索引
fn find_node_by_byte(tree: &Tree, byte: usize) -> Option<usize> {
    let nodes = collect_nodes_dfs(&tree.root);
    nodes.iter().position(|n| n.title_byte_range.contains(&byte))
}

fn collect_nodes_dfs(node: &Node) -> Vec<&Node> {
    let mut v = vec![node];
    for child in &node.children {
        v.extend(collect_nodes_dfs(child));
    }
    v
}
```

- [ ] **Step 2: 实现 Indent（降级）**

```rust
fn exec_indent(tree: &Tree, node_idx: usize, doc: &mut dyn DocViewMut) {
    let nodes = collect_nodes_dfs(&tree.root);
    let node = &nodes[node_idx];
    if node.heading_level >= 6 { return; } // 最大六级

    // 查找前一个同级节点（同 parent 下的前一个 child）
    let parent = find_parent(&tree.root, node_idx);
    let siblings = parent.map(|p| &p.children).unwrap_or(&vec![]);

    let current_sibling_pos = siblings.iter()
        .position(|s| s.title_byte_range == node.title_byte_range);

    let target_parent = match current_sibling_pos {
        Some(0) | None => return, // 第一个同级无法降级
        Some(pos) => &siblings[pos - 1],
    };

    // 在 node.title_byte_range.start 处插入 "#"
    doc.begin_edit();
    doc.replace_range(node.title_byte_range.start..node.title_byte_range.start, "#");
    // 对所有子节点各加 "#"
    for child in collect_nodes_dfs(node).iter().skip(1) {
        doc.replace_range(child.title_byte_range.start..child.title_byte_range.start, "#");
    }
    doc.end_edit();
}

fn find_parent<'a>(root: &'a Node, target_idx: usize) -> Option<&'a Node> {
    let nodes = collect_nodes_dfs(root);
    let target = nodes.get(target_idx)?;
    find_parent_of(root, target)
}

fn find_parent_of<'a>(node: &'a Node, target: &Node) -> Option<&'a Node> {
    for child in &node.children {
        if std::ptr::eq(child as *const Node, target as *const Node) {
            return Some(node);
        }
        if let Some(p) = find_parent_of(child, target) {
            return Some(p);
        }
    }
    None
}
```

- [ ] **Step 3: 实现 Outdent（升级）**

```rust
fn exec_outdent(tree: &Tree, node_idx: usize, doc: &mut dyn DocViewMut) {
    let nodes = collect_nodes_dfs(&tree.root);
    let node = &nodes[node_idx];
    if node.heading_level <= 1 { return; } // 根节点不能升级

    // 删除标题前的一个 "#"
    let hash_pos = node.title_byte_range.start;
    // 找到标题前的 "#"（在 hash_pos 之前的一个字节）
    if hash_pos == 0 { return; }
    doc.begin_edit();
    doc.replace_range(hash_pos - 1..hash_pos, "");
    // 对所有子节点各删除一个 "#"
    for child in collect_nodes_dfs(node).iter().skip(1) {
        let child_hash = child.title_byte_range.start;
        if child_hash > 0 {
            doc.replace_range(child_hash - 1..child_hash, "");
        }
    }
    doc.end_edit();
}
```

- [ ] **Step 4: 实现 NewSibling / NewChild**

```rust
fn exec_new_sibling(tree: &Tree, node_idx: usize, doc: &mut dyn DocViewMut) {
    let nodes = collect_nodes_dfs(&tree.root);
    let node = &nodes[node_idx];
    let level = node.heading_level;
    // 在节点 source_range.end 后插入新行
    let insert_pos = node.source_range.end;
    let hashes = "#".repeat(level as usize);
    let new_line = format!("\n{} 新节点\n", hashes);
    doc.begin_edit();
    doc.replace_range(insert_pos..insert_pos, &new_line);
    doc.end_edit();
}

fn exec_new_child(tree: &Tree, node_idx: usize, doc: &mut dyn DocViewMut) {
    let nodes = collect_nodes_dfs(&tree.root);
    let node = &nodes[node_idx];
    let level = node.heading_level + 1;
    let hashes = "#".repeat(level as usize);
    // 在节点正文末尾插入子节点
    let insert_pos = if node.children.is_empty() {
        node.title_byte_range.end
    } else {
        node.children.last().unwrap().source_range.end
    };
    let new_line = format!("\n{} 新子节点\n", hashes);
    doc.begin_edit();
    doc.replace_range(insert_pos..insert_pos, &new_line);
    doc.end_edit();
}
```

- [ ] **Step 5: 实现 handle_intercept_key 入口**

```rust
pub fn handle_intercept_key(
    key: KeyCode,
    modifiers: Modifiers,
    tree: &Tree,
    focus_byte: usize,
    doc: &mut dyn DocViewMut,
) -> bool {
    let node_idx = match find_node_by_byte(tree, focus_byte) {
        Some(idx) => idx,
        None => return false, // 光标不在任何节点内
    };

    match (key, modifiers) {
        (KeyCode::Tab, Modifiers::NONE) => {
            exec_indent(tree, node_idx, doc);
            true
        }
        (KeyCode::Tab, Modifiers { shift: true, .. }) => {
            exec_outdent(tree, node_idx, doc);
            true
        }
        (KeyCode::Enter, Modifiers::NONE) => {
            exec_new_sibling(tree, node_idx, doc);
            true
        }
        (KeyCode::Enter, Modifiers { ctrl: true, .. }) => {
            exec_new_child(tree, node_idx, doc);
            true
        }
        _ => false,
    }
}
```

- [ ] **Step 6: 更新 mmf/mod.rs**

```rust
pub mod edit;
```

- [ ] **Step 7: Commit**

```bash
git add crates/markdown/src/mmf/
git commit -m "feat(mmf): InterceptKey 结构编辑——Indent/Outdent/NewSibling/NewChild

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 7: MindmapView + MindmapPluginFactory + 注册

**Files:**
- Create: `crates/markdown/src/mindmap_view.rs`
- Modify: `crates/markdown/src/lib.rs`
- Modify: `crates/app/src/workspace.rs` (registration)

**Interfaces:**
- Implements: `ui::plugin::ViewPlugin` for `MindmapView`
- Implements: `ui::plugin::PluginFactory` for `MindmapPluginFactory`

- [ ] **Step 1: 定义 MindmapView struct**

在 `crates/markdown/src/mindmap_view.rs`：

```rust
use std::path::Path;
use core::document::{DocView, DocViewMut};
use shaping::Shaper;
use ui::core::geom::Rect;
use ui::core::paint::DrawList;
use ui::plugin::{
    ViewPlugin, PluginFactory, PluginMessage, PluginQuery, PluginResponse,
    Direction, HitResult,
};
use ui::theme::Theme;
use crate::mmf::{
    self, Tree,
    layout::{LayoutTree, LayoutConstants, HitMap},
};

pub struct MindmapView {
    source: Option<String>,
    cached_generation: u32,
    tree: Option<Tree>,
    layout_tree: Option<LayoutTree>,
    hit_map: Option<HitMap>,
    node_titles: Vec<String>,        // DFS-ordered titles（layout 期间填充）
    scroll_y: f32,
    focus_byte: Option<usize>,        // 当前焦点光标字节偏移
    constants: LayoutConstants,
}

impl MindmapView {
    pub fn new() -> Self {
        Self {
            source: None,
            cached_generation: 0,
            tree: None,
            layout_tree: None,
            hit_map: None,
            node_titles: Vec::new(),
            scroll_y: 0.0,
            focus_byte: None,
            constants: LayoutConstants::default(),
        }
    }
}
```

- [ ] **Step 2: 实现 query 辅助方法**

```rust
impl MindmapView {
    fn ensure_layout(&mut self, shaper: &mut Shaper) {
        if self.layout_tree.is_some() { return; }
        let tree = match &self.tree {
            Some(t) => t,
            None => return,
        };
        let lt = mmf::layout::compute_layout(tree, shaper, &self.constants);
        let hm = mmf::layout::build_hit_map(tree, &lt, shaper, &self.constants);
        // 收集 DFS 序的节点标题（供 canvas 渲染用）
        self.node_titles = collect_dfs_titles(&tree.root);
        self.layout_tree = Some(lt);
        self.hit_map = Some(hm);
    }

    fn hit_test(&self, canvas_x: f32, canvas_y: f32) -> Option<HitResult> {
        let hm = self.hit_map.as_ref()?;
        let lt = self.layout_tree.as_ref()?;
        let tree = self.tree.as_ref()?;
        let nodes = collect_nodes_dfs(&tree.root);

        for (i, rect) in hm.node_rects.iter().enumerate() {
            if rect.contains(canvas_x, canvas_y) {
                // 找准确的字符偏移
                let edges = hm.title_char_edges.get(i)?;
                let node = nodes.get(i)?;
                let text_x = rect.x + self.constants.card_padding_x;
                let char_idx = edges.iter()
                    .position(|&edge| canvas_x < edge)
                    .unwrap_or(edges.len().saturating_sub(1));
                let byte_offset = if node.title.is_empty() {
                    node.title_byte_range.start
                } else {
                    // char_idx → byte offset
                    node.title.char_indices()
                        .nth(char_idx)
                        .map(|(b, _)| node.title_byte_range.start + b)
                        .unwrap_or(node.title_byte_range.end)
                };
                return Some(HitResult { byte_offset, node_idx: i });
            }
        }
        None
    }

    fn cursor_rect(&self, byte_offset: usize) -> Option<(f32, f32, f32)> {
        let hm = self.hit_map.as_ref()?;
        let lt = self.layout_tree.as_ref()?;
        let tree = self.tree.as_ref()?;
        let nodes = collect_nodes_dfs(&tree.root);

        for (i, node) in nodes.iter().enumerate() {
            if !node.title_byte_range.contains(&byte_offset) { continue; }
            let ln = &lt.nodes[i];
            let edges = hm.title_char_edges.get(i)?;
            let char_idx = if node.title.is_empty() {
                0
            } else {
                let rel_byte = byte_offset - node.title_byte_range.start;
                node.title[..rel_byte.min(node.title.len())].chars().count()
            };
            let x = edges.get(char_idx).copied().unwrap_or(ln.x + self.constants.card_padding_x);
            let y = ln.y + self.constants.card_padding_y;
            let h = self.constants.card_height - 2.0 * self.constants.card_padding_y;
            return Some((x, y, h));
        }
        None
    }

    fn visual_move(&self, from_byte: usize, direction: Direction) -> Option<usize> {
        let tree = self.tree.as_ref()?;
        let nodes = collect_nodes_dfs(&tree.root);

        let (node_idx, pos_in_title) = nodes.iter()
            .enumerate()
            .find(|(_, n)| n.title_byte_range.contains(&from_byte))
            .map(|(i, n)| (i, from_byte - n.title_byte_range.start))?;

        let node = nodes[node_idx];

        match direction {
            Direction::Up | Direction::Down => {
                // 找同级节点
                let siblings = find_siblings(tree, node_idx)?;
                let sib_idx = siblings.iter().position(|&si| si == node_idx)?;
                let target = match direction {
                    Direction::Up => siblings.get(sib_idx.checked_sub(1)?),
                    Direction::Down => siblings.get(sib_idx + 1),
                    _ => None,
                }?;
                let target_node = nodes.get(*target)?;
                // 尽量保持相近的列位置
                let char_idx = pos_in_title.min(target_node.title.len());
                let byte_off = target_node.title_byte_range.start
                    + target_node.title[..char_idx].len();
                Some(byte_off)
            }
            Direction::Left => {
                if pos_in_title == 0 {
                    // 跳转父节点
                    let parent = find_parent(tree, node_idx)?;
                    Some(parent.title_byte_range.start)
                } else {
                    // 左移一个字符
                    let prev_char = node.title[..pos_in_title.min(node.title.len())]
                        .chars().next_back()?;
                    Some(from_byte - prev_char.len_utf8())
                }
            }
            Direction::Right => {
                let title_len = node.title.len();
                if pos_in_title >= title_len {
                    // 跳转第一个子节点
                    let child = node.children.first()?;
                    Some(child.title_byte_range.start)
                } else {
                    // 右移一个字符
                    let next_char = node.title[pos_in_title..].chars().next()?;
                    Some(from_byte + next_char.len_utf8())
                }
            }
        }
    }
}
```

- [ ] **Step 3: 实现 ViewPlugin trait**

```rust
impl ViewPlugin for MindmapView {
    fn name(&self) -> &str { "mindmap" }

    fn render(
        &mut self,
        doc: &dyn DocView,
        bounds: Rect,
        theme: &Theme,
        shaper: &mut Shaper,
        dpi_scale: f32,
    ) -> DrawList {
        // 确保已解析 + 布局
        self.ensure_layout(shaper);

        let mut dl = DrawList::new();
        let lt = match &self.layout_tree {
            Some(lt) => lt,
            None => return dl,
        };

        // 视口裁剪 + 渲染
        mmf::canvas::render(
            &mut dl, lt, bounds, theme, &self.constants,
            shaper, &self.node_titles,
        );

        dl
    }

    fn handle_message(&mut self, msg: PluginMessage, doc: &mut dyn DocViewMut) -> bool {
        match msg {
            PluginMessage::UpdateSource { text, generation } => {
                self.source = Some(text.clone());
                self.cached_generation = generation;
                // 重新解析，清空布局缓存
                self.tree = mmf::parser::parse(&text).ok();
                self.layout_tree = None;
                self.hit_map = None;
                true
            }
            PluginMessage::InterceptKey { key, modifiers } => {
                let tree = match &self.tree {
                    Some(t) => t,
                    None => return false,
                };
                let fb = self.focus_byte.unwrap_or(0);
                mmf::edit::handle_intercept_key(key, modifiers, tree, fb, doc)
            }
            PluginMessage::SetScrollY(y) => {
                self.scroll_y = y;
                true
            }
            PluginMessage::SetSelCursor(pos) => {
                if let Some((line, col)) = pos {
                    // line,col → byte offset
                    let byte = self.line_col_to_byte(doc, line, col);
                    self.focus_byte = Some(byte);
                }
                true
            }
            _ => false,
        }
    }

    fn query(&self, query: PluginQuery, doc: &dyn DocView) -> PluginResponse {
        match query {
            PluginQuery::HitTestCanvas { x, y, offset_x, offset_y } => {
                PluginResponse::HitResult(self.hit_test(x - offset_x, y - offset_y))
            }
            PluginQuery::CursorRect(byte_offset) => {
                PluginResponse::CursorRect(self.cursor_rect(byte_offset))
            }
            PluginQuery::VisualMove { from_byte, direction } => {
                PluginResponse::Position(self.visual_move(from_byte, direction)
                    .map(|b| self.byte_to_line_col(doc, b)))
            }
            PluginQuery::ContentHeight => {
                let h = self.layout_tree.as_ref()
                    .map(|lt| lt.total_h).unwrap_or(0.0);
                PluginResponse::Float(h)
            }
            PluginQuery::ScrollY => PluginResponse::Float(self.scroll_y),
            _ => PluginResponse::None,
        }
    }

    fn shows_cursor(&self) -> bool { false }
    fn shows_gutter(&self) -> bool { false }
    fn allows_editing(&self) -> bool { true }
}
```

还需要两个辅助方法 `line_col_to_byte` / `byte_to_line_col`：

```rust
impl MindmapView {
    fn line_col_to_byte(&self, doc: &dyn DocView, line: usize, col: usize) -> usize {
        doc.line_byte_offset(line) + col
    }

    fn byte_to_line_col(&self, doc: &dyn DocView, byte: usize) -> (usize, usize) {
        // 遍历 lines 找到包含 byte 的行
        for l in 0..doc.line_count() {
            let start = doc.line_byte_offset(l);
            let len = doc.line_byte_length(l);
            if byte >= start && byte <= start + len {
                return (l, byte - start);
            }
        }
        (0, 0)
    }
}
```

- [ ] **Step 4: 实现 PluginFactory**

```rust
pub struct MindmapPluginFactory;

impl PluginFactory for MindmapPluginFactory {
    fn name(&self) -> &str { "mindmap" }

    fn can_handle(&self, path: Option<&Path>) -> bool {
        path.and_then(|p| p.to_str())
            .is_some_and(|s| s.ends_with(".mmap.md"))
    }

    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(MindmapView::new())
    }
}
```

- [ ] **Step 5: 在 lib.rs 中导出**

在 `crates/markdown/src/lib.rs` 添加：

```rust
pub mod mmf;
pub mod mindmap_view;
```

- [ ] **Step 6: 在 workspace.rs 中注册**

在 `crates/app/src/workspace.rs` 第 94 行附近（其他 `registry.register()` 调用之后）添加：

```rust
registry.register(Box::new(edit_plus_markdown::mindmap_view::MindmapPluginFactory));
```

- [ ] **Step 7: cargo check 验证**

```bash
cargo check 2>&1
```

修复任何编译错误。确认 `cargo check` 通过。

- [ ] **Step 8: Commit**

```bash
git add crates/markdown/src/mindmap_view.rs crates/markdown/src/lib.rs crates/app/src/workspace.rs
git commit -m "feat(mindmap): MindmapView + MindmapPluginFactory + app 注册

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 8: 端到端集成验证

**Files:** 无新建，整体验证。

- [ ] **Step 1: 确认 full workspace 编译通过**

```bash
cargo build 2>&1 | tail -20
```

- [ ] **Step 2: 运行所有现有测试确保无回归**

```bash
cargo test 2>&1 | tail -30
```

- [ ] **Step 3: 运行 markdown crate 测试**

```bash
cargo test -p edit-plus-markdown 2>&1 | tail -20
```

- [ ] **Step 4: 创建 .mmap.md 测试文件**

创建 `test_data/sample.mmap.md`：

```markdown
```toml mindmap
version = 1
layout = "auto"
```

# 产品规划

## 数据同步

```toml node
priority = "P1"
status = "todo"
```

需要支持本地文件、云端同步和冲突解决。

### 本地文件

### 云端同步

## AI 生成

支持从 Prompt 生成导图。
```

- [ ] **Step 5: 手动启动验证**

```bash
cargo run -- test_data/sample.mmap.md
```

检查：
- 思维导图树布局是否正确渲染（画布可见卡片/连线）
- 源码视图切换是否正常（切换到源码应看到原始 MMF）
- 点击卡片是否能定位光标
- Tab/Enter 结构编辑是否能正确修改 MMF 源码

- [ ] **Step 6: Commit final fixes**

```bash
git add test_data/
git commit -m "test: 添加 MMF 集成测试数据

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## 依赖关系

```
Task 1 (Plugin 扩展) ─────────────────────┐
Task 2 (DocViewMut 扩展) ─────────────────┤
Task 3 (Model + Parser) ──┬─ Task 4 (Layout) ── Task 5 (Canvas) ─┐
                          │                                        │
                          └─ Task 6 (Edit) ────────────────────────┤
                                                                   │
                          ┌────────────────────────────────────────┘
                          └─ Task 7 (MindmapView + Registration)

Task 8 (集成验证) ← depends on all
```

Task 1 和 Task 2 可并行；Task 3 独立；Task 4/5/6 依赖 Task 3 且 6 依赖 2；Task 7 依赖 1+2+3+4+5+6。
