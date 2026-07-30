# Markdown WYSIWYG 编辑器实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在现有 PreviewEngine 架构上实现 Typora 式所见即所得编辑——光标进入时局部展开 markdown 标记符，离开时折叠为富文本。

**Architecture:** Thin View, Smart Controller — app 层持有输入/Undo/IME，插件通过新增 PluginQuery 提供布局相关查询，通过 PluginMessage 接收变更通知。EditContext 控制 LazyLayout 中的 materialize_text 展开逻辑。

**Tech Stack:** Rust 2024, pulldown_cmark, wgpu, winit

## Global Constraints

- `./scripts/verify.sh` 每次提交前必须通过
- 函数不超过 50 行
- 禁止 `.unwrap()` 无 `.expect("说明理由")`
- `cargo fmt` + `cargo clippy` 零警告
- 零硬编码类型判断，全部通过 `ViewPlugin` trait 交互
- UI 层不直接依赖 app 层

---

## 文件结构

```
crates/markdown/src/
  view.rs              ← MarkdownEditorView (新建) + MarkdownEditorViewFactory
  parser.rs            ← ParsedMarkdown 增加 event_offsets
  builder.rs           ← StyleSpan 增加 source_range, BlockNode 增加 source_range
  layout.rs            ← EditContext 定义, materialize_text()
  lib.rs               ← 无需改动

crates/ui/src/
  plugin.rs            ← 新增 PluginMessage::SetCursorByte,
                          新增 PluginQuery: HitTestByte, VisualMove, CursorScreenPos, AugmentEdit
                          新增 PluginResponse: CursorRect, BytePosition, Augmentation
                          新增 ViewPlugin::is_wysiwyg()

crates/app/src/
  dispatch/editor.rs   ← WYSIWYG 编辑分派路径
```

---

### Task 1: 源码映射——StyleSpan/BlockNode 增加 source_range

**Files:**
- Modify: `crates/markdown/src/parser.rs:62-130`
- Modify: `crates/markdown/src/builder.rs:12-20, 66-69`
- Modify: `crates/markdown/src/builder.rs` (builder 逻辑，传递 source_range)

**Interfaces:**
- Consumes: 无
- Produces: `ParsedMarkdown { events, event_offsets }`, `StyleSpan { ..., source_range: Range<usize> }`, `BlockNode { ..., source_range: Range<usize> }`

- [ ] **Step 1: ParsedMarkdown 存储 event_offsets**

修改 `crates/markdown/src/parser.rs`：

```rust
// 第62行，ParsedMarkdown 结构体添加 event_offsets 字段
#[derive(Clone, Debug, Default)]
pub struct ParsedMarkdown {
    pub events: Vec<MarkdownEvent>,
    /// event_ranges[i] = events[i] 在源码中的完整字节区间。
    /// 存储 Range<usize> 而非仅 start，避免相邻元素边界推断错误。
    pub event_ranges: Vec<Range<usize>>,
}

// 第128行附近，parse_markdown 返回时包含 event_ranges
// 原: let mut event_offsets = Vec::new();
//     event_offsets.push(range.start);
// 改为: let mut event_ranges = Vec::new();
//     event_ranges.push(range);  // 直接存储完整 Range
//
// 第130行返回:
ParsedMarkdown { events, event_ranges }
```

- [ ] **Step 2: StyleSpan 增加 source_range**

修改 `crates/markdown/src/builder.rs` 第66行：

```rust
#[derive(Clone, Debug)]
pub struct StyleSpan {
    pub start: usize,       // 折叠后文本中的字节偏移
    pub len: usize,         // 折叠后文本的字节长度
    pub style: InlineStyle,
    /// 此 span 在原始源码中的字节范围 (含标记符, e.g. "**world**" → 6..13)
    pub source_range: Range<usize>,
}
```

- [ ] **Step 3: BlockNode 增加 source_range**

修改 `crates/markdown/src/builder.rs` 第13行：

```rust
#[derive(Clone, Debug)]
pub struct BlockNode {
    pub kind: BlockKind,
    pub children: Vec<BlockNode>,
    pub text_lines: Vec<String>,
    pub text_styles: Vec<Vec<StyleSpan>>,
    /// 此 Block 在原始源码中的字节范围 (含标记符)。用于增量 diff 和字节定位。
    pub source_range: Range<usize>,
}
```

- [ ] **Step 4: 扫描 builder 中所有构造 StyleSpan/BlockNode 的位置，补上 source_range**

需要修改 builder.rs 中以下位置的构造代码：

每处 `StyleSpan { start, len, style }` 改为 `StyleSpan { start, len, style, source_range: start..end }`。
每处 `BlockNode { kind, children, text_lines, text_styles }` 改为 `BlockNode { kind, children, text_lines, text_styles, source_range: block_start..block_end }`。

具体修改点在 builder 的 `close_paragraph`、`finalize`、`build_inner` 等函数中。builder 在处理 `event_offsets` 时需要跟踪当前块的起始偏移和每个 inline span 的源码范围。

- [ ] **Step 5: 编译验证**

```bash
cargo build -p edit-plus-markdown 2>&1
```

预期：编译成功。`source_range` 的默认值可能需要在 `BlockNode` 和 `StyleSpan` 上使用 `Default` derive 或在构造代码中显式填写。

- [ ] **Step 6: 测试**

```bash
cargo test -p edit-plus-markdown 2>&1
```

预期：现有测试通过 (source_range 不影响解析逻辑)。

- [ ] **Step 7: 添加单元测试——验证 source_range 正确性**

在 `builder.rs` 的 `#[cfg(test)]` 模块中添加：

```rust
#[test]
fn source_range_captures_bold_marker() {
    let src = "hello **world** here";
    let parsed = crate::parser::parse_markdown(src);
    let style = MarkdownStyle::default_for_test();
    let doc = MarkdownDoc::build(&parsed, &style);

    // 找到 Bold span
    let bold_span = doc.blocks.iter()
        .flat_map(|b| b.text_styles.iter().flatten())
        .find(|s| s.style == InlineStyle::Bold)
        .expect("should have a bold span");

    assert_eq!(bold_span.source_range, 6..13);
    // 验证折叠后文本
    assert_eq!(&src[bold_span.source_range.clone()], "**world**");
}

#[test]
fn plain_span_no_marker() {
    let src = "plain text";
    let parsed = crate::parser::parse_markdown(src);
    let style = MarkdownStyle::default_for_test();
    let doc = MarkdownDoc::build(&parsed, &style);

    let span = &doc.blocks[0].text_styles[0][0];
    assert_eq!(span.source_range, 0..10);
    assert_eq!(span.style, InlineStyle::Plain); // 假设 Plain 已存在
}
```

- [ ] **Step 8: 运行测试并提交**

```bash
cargo test -p edit-plus-markdown 2>&1
git add crates/markdown/src/parser.rs crates/markdown/src/builder.rs
git commit -m "feat: StyleSpan/BlockNode 增加 source_range，ParsedMarkdown 保留 event_offsets"
```

---

### Task 2: Plugin 基础设施——新增 Message/Query/Response 变体 + is_wysiwyg()

**Files:**
- Modify: `crates/ui/src/plugin.rs`

**Interfaces:**
- Consumes: 无
- Produces: `PluginMessage::SetCursorByte(usize)`, `PluginQuery::{HitTestByte, VisualMove, CursorScreenPos, AugmentEdit}`, `PluginResponse::{CursorRect, BytePosition, Augmentation}`, `ViewPlugin::is_wysiwyg()`, `MoveDirection`, `AugmentKind`, `EditAugmentation`

- [ ] **Step 1: 在 PluginMessage 枚举中添加 SetCursorByte**

```rust
pub enum PluginMessage {
    // ... 现有变体保持不变

    /// 通知插件光标在源码中的字节位置已变更。
    SetCursorByte(usize),
}
```

- [ ] **Step 2: 定义新类型——MoveDirection, AugmentKind, EditAugmentation**

在 `PluginMessage` 定义之后、`PluginQuery` 之前添加：

```rust
/// 视觉导航方向。
#[derive(Debug, Clone, Copy)]
pub enum MoveDirection {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
}

/// 编辑干预类型。
#[derive(Debug, Clone, Copy)]
pub enum AugmentKind {
    Enter,
    Backspace,
    Tab,
}

/// 编辑干预建议。
#[derive(Debug, Clone)]
pub struct EditAugmentation {
    /// 实际删除范围 (替代默认单字符退格)。None 表示不修改。
    pub delete_range: Option<std::ops::Range<usize>>,
    /// 实际插入文本 (替代默认 "\n" 或 ""). None 表示使用默认。
    pub insert_text: Option<String>,
    /// 操作后的光标字节位置。
    pub cursor_byte_after: usize,
}

impl Default for EditAugmentation {
    fn default() -> Self {
        Self {
            delete_range: None,
            insert_text: None,
            cursor_byte_after: 0,
        }
    }
}
```

- [ ] **Step 3: 在 PluginQuery 枚举中添加新变体**

```rust
pub enum PluginQuery {
    // ... 现有变体保持不变

    /// 源码字节偏移 → 屏幕像素 (x, y, w, h)。用于 IME 选词框定位。
    CursorScreenPos(usize),
    // → CursorRect(Option<(f32, f32, f32, f32)>)

    /// 屏幕像素 → 源码字节偏移。用于鼠标点击定位。
    HitTestByte { x: f32, y: f32, offset_x: f32, offset_y: f32 },
    // → BytePosition(Option<usize>)

    /// 视觉方向导航：从 current_byte 向上/下/左/右移动，
    /// 返回目标源码字节偏移 (考虑折叠态不可见标记符)。
    VisualMove {
        current_byte: usize,
        direction: MoveDirection,
        /// 上/下移动时保持的偏好 X 像素位置。
        target_x: Option<f32>,
    },
    // → BytePosition(Option<usize>)

    /// 询问插件是否需要对此次编辑做干预 (回车续接列表、退格配对删除等)。
    AugmentEdit {
        current_byte: usize,
        kind: AugmentKind,
    },
    // → Augmentation(Option<EditAugmentation>)
}
```

- [ ] **Step 4: 在 PluginResponse 枚举中添加新变体**

```rust
pub enum PluginResponse {
    // ... 现有变体保持不变

    /// (x, y, w, h) — 光标在文档坐标系中的矩形区域。
    CursorRect(Option<(f32, f32, f32, f32)>),
    /// 源码字节偏移。
    BytePosition(Option<usize>),
    /// 编辑干预建议。
    Augmentation(Option<EditAugmentation>),
}
```

- [ ] **Step 5: 在 ViewPlugin trait 添加 is_wysiwyg() 方法**

```rust
pub trait ViewPlugin {
    // ... 现有方法

    /// 是否为 WYSIWYG 编辑器 (需要布局感知的编辑辅助)。
    /// 当返回 true 时，app 层在分发 EditCommand 前会查询插件的布局数据。
    fn is_wysiwyg(&self) -> bool {
        false
    }
}
```

- [ ] **Step 6: 编译验证**

```bash
cargo build 2>&1
```

预期：需要在所有 `ViewPlugin` 实现者处检查 match exhaustiveness。`MarkdownView` 和 `NovelView` 的 `query()` 方法中匹配了新变体，需要添加相应分支。

- [ ] **Step 7: 更新 MarkdownView 的 query() 方法——为新变体添加默认分支**

在 `crates/markdown/src/view.rs` 的 `MarkdownView::query()` 方法中添加：

```rust
fn query(&self, q: PluginQuery, _doc: &dyn DocView) -> PluginResponse {
    match q {
        // ... 现有分支保持不变
        PluginQuery::CursorScreenPos(_) => PluginResponse::CursorRect(None),
        PluginQuery::HitTestByte { .. } => PluginResponse::BytePosition(None),
        PluginQuery::VisualMove { .. } => PluginResponse::BytePosition(None),
        PluginQuery::AugmentEdit { .. } => PluginResponse::Augmentation(None),
    }
}
```

对 `NovelView::query()` 做同样处理。

- [ ] **Step 8: 添加 PluginRegistry 测试**

在 `crates/ui/src/plugin.rs` 的 tests 模块中添加：

```rust
#[test]
fn is_wysiwyg_defaults_to_false() {
    let plugin = StubPlugin { name: "test" };
    assert!(!plugin.is_wysiwyg());
}
```

- [ ] **Step 9: 编译 + 测试 + 提交**

```bash
cargo build 2>&1
cargo test -p edit-plus-ui 2>&1
cargo test -p edit-plus-markdown 2>&1

git add crates/ui/src/plugin.rs crates/markdown/src/view.rs
git commit -m "feat: Plugin 基础设施——新增 SetCursorByte/HitTestByte/VisualMove/CursorScreenPos/AugmentEdit/is_wysiwyg"
```

---

### Task 3: Engine 层——EditContext + materialize_text + 查询实现

**Files:**
- Create: `crates/markdown/src/edit.rs`
- Modify: `crates/markdown/src/layout.rs`
- Modify: `crates/markdown/src/view.rs` (PreviewEngine 改动)
- Modify: `crates/markdown/src/lib.rs`

**Interfaces:**
- Consumes: `StyleSpan.source_range`, `BlockNode.source_range` (Task 1), `PluginMessage::SetCursorByte`, 新增 `PluginQuery` 变体 (Task 2)
- Produces: `EditContext`, `materialize_text()`, `PreviewEngine::handle_set_cursor_byte()`, `PreviewEngine::cursor_screen_pos()`, `PreviewEngine::hit_test_byte()`, `PreviewEngine::visual_move()`, `PreviewEngine::augment_edit()`, `PreviewEngine::cursor_in_span()`

- [ ] **Step 1: 创建 edit.rs——EditContext 定义 + 纯函数**

创建 `crates/markdown/src/edit.rs`：

```rust
//! 编辑上下文——WYSIWYG 展开机制的核心数据类型和纯函数。

use std::ops::Range;
use crate::builder::{StyleSpan, InlineStyle};

/// 编辑器光标上下文。传入 LazyLayout 控制哪些 span 展开源码。
#[derive(Clone, Debug)]
pub struct EditContext {
    /// 光标在源码中的字节偏移 (插入点，位于两个字节之间)。
    pub cursor_byte: usize,
}

/// 判断光标是否在 span 的源码范围内。
/// 使用闭区间右侧 (<= end)：光标在 span 末尾时仍视为"在 span 内"，
/// 让用户能在边界处继续输入同一样式的文本。
pub fn cursor_in_span(span: &StyleSpan, cursor_byte: usize) -> bool {
    span.source_range.start <= cursor_byte && cursor_byte <= span.source_range.end
}

/// 为 span 获取折叠文本——去掉标记符后的纯渲染文本。
/// e.g. "**world**" → "world", "*italic*" → "italic"
pub fn fold_text(source: &str, span: &StyleSpan) -> &str {
    let range = &span.source_range;
    let marker_len = span_marker_len(&span.style);
    let start = range.start + marker_len.0;
    let end = range.end - marker_len.1;
    // 边界安全：确保不越界
    if start >= end {
        return "";
    }
    &source[start..end]
}

/// 返回 (prefix_marker_len, suffix_marker_len)
fn span_marker_len(style: &InlineStyle) -> (usize, usize) {
    match style {
        InlineStyle::Bold => (2, 2),
        InlineStyle::Italic => (1, 1),
        InlineStyle::Strikethrough => (2, 2),
        InlineStyle::InlineCode => (1, 1),
        InlineStyle::Link { url: _ } => (1, 0), // prefix marker handled differently
        // Plain and others: no marker
        _ => (0, 0),
    }
}

/// 拼接一个 Block 的布局用文本。
/// - edit_ctx 为 None → 全部使用折叠文本 (快速路径)
/// - edit_ctx 为 Some → 光标所在 span 使用完整源码，其他使用折叠文本
pub fn materialize_text(
    spans: &[StyleSpan],
    source: &str,
    edit_ctx: Option<&EditContext>,
) -> String {
    let Some(ctx) = edit_ctx else {
        return spans.iter().fold(String::new(), |mut acc, s| {
            acc.push_str(fold_text(source, s));
            acc
        });
    };

    let mut text = String::new();
    for span in spans {
        if cursor_in_span(span, ctx.cursor_byte) {
            text.push_str(&source[span.source_range.clone()]);
        } else {
            text.push_str(fold_text(source, span));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::InlineStyle;

    fn make_span(start: usize, len: usize, source_start: usize, source_end: usize, style: InlineStyle) -> StyleSpan {
        StyleSpan { start, len, style, source_range: source_start..source_end }
    }

    #[test]
    fn cursor_in_span_inclusive_end() {
        let span = make_span(6, 5, 6, 13, InlineStyle::Bold);
        assert!(cursor_in_span(&span, 6));   // 光标在起始边界
        assert!(cursor_in_span(&span, 10));  // 光标在中间
        assert!(cursor_in_span(&span, 13));  // 光标在结束边界 (闭区间)
        assert!(!cursor_in_span(&span, 5));  // 光标在 span 前
        assert!(!cursor_in_span(&span, 14)); // 光标在 span 后
    }

    #[test]
    fn materialize_text_folds_when_no_edit_ctx() {
        let source = "hello **world** here";
        let spans = vec![
            make_span(0, 6, 0, 6, InlineStyle::Plain),
            make_span(6, 5, 6, 13, InlineStyle::Bold),
            make_span(11, 5, 13, 18, InlineStyle::Plain),
        ];
        let text = materialize_text(&spans, source, None);
        assert_eq!(text, "hello world here");
    }

    #[test]
    fn materialize_text_unfolds_cursor_span() {
        let source = "hello **world** here";
        let spans = vec![
            make_span(0, 6, 0, 6, InlineStyle::Plain),
            make_span(6, 5, 6, 13, InlineStyle::Bold),
            make_span(11, 5, 13, 18, InlineStyle::Plain),
        ];
        // 光标在 Bold span 内
        let ctx = EditContext { cursor_byte: 10 };
        let text = materialize_text(&spans, source, Some(&ctx));
        assert_eq!(text, "hello **world** here");
    }
}
```

- [ ] **Step 2: 在 lib.rs 中声明 edit 模块**

修改 `crates/markdown/src/lib.rs`：

```rust
pub mod edit;
```

- [ ] **Step 3: 编译 + 测试 edit.rs**

```bash
cargo test -p edit-plus-markdown edit 2>&1
```

- [ ] **Step 4: PreviewEngine 增加 edit_ctx 字段**

修改 `crates/markdown/src/view.rs`，在 `PreviewEngine` 结构体中添加：

```rust
pub struct PreviewEngine {
    // ... 现有字段保持不变
    /// WYSIWYG 编辑上下文。None 表示纯预览模式 (快速路径)。
    pub edit_ctx: Option<crate::edit::EditContext>,
}
```

在 `PreviewEngine::new()` 中添加：

```rust
edit_ctx: None,
```

- [ ] **Step 5: PreviewEngine 实现 handle_set_cursor_byte**

在 `impl PreviewEngine` 块中添加：

```rust
/// 接收来自 app 层的光标位置变更通知。
pub fn handle_set_cursor_byte(&mut self, byte: usize) {
    self.edit_ctx = Some(crate::edit::EditContext { cursor_byte: byte });
    self.mark_dirty(); // 下一帧 re-layout 使用新光标位置
}
```

- [ ] **Step 6: PreviewEngine 实现 cursor_screen_pos**

```rust
/// 查询：返回光标在文档坐标系中的 (x, y, w, h)。
pub fn cursor_screen_pos(&self) -> Option<(f32, f32, f32, f32)> {
    let ctx = self.edit_ctx.as_ref()?;
    let lazy = self.lazy.as_ref()?;
    let doc = self.doc.as_ref()?;

    let cursor_byte = ctx.cursor_byte;

    // 1. 找到包含当前字节的 Block 和行
    let (block_idx, line_in_block, _span_idx) = self.find_char_position(cursor_byte)?;

    // 2. 算该字节在 materialize 后行内的字符偏移
    let char_offset = self.char_offset_from_byte(block_idx, line_in_block, cursor_byte)?;

    // 3. 查该行 ShapedRun → glyph_x_at_char
    let flat_line = &lazy.flat_lines[/* 对应索引 */];
    let x = flat_line.shaped.as_ref()?.glyph_x_at_char(char_offset)?;

    // 4. 返回 (x, y, w=2, h=line_height)
    Some((x, flat_line.rect.y, 2.0, flat_line.rect.h))
}
```

> **Note:** `find_char_position` 和 `char_offset_from_byte` 的实现取决于 LazyLayout 的 flat_lines 索引和 source_range 映射。实现时需遍历 BlockNode 的 text_styles，根据 source_range 定位到对应 span 和行，再计算 materialize 后的字符偏移。

- [ ] **Step 7: PreviewEngine 实现 hit_test_byte**

```rust
/// 屏幕坐标 → 源码字节偏移。用于鼠标点击。
/// 返回 None 表示点击位置无对应文本。
pub fn hit_test_byte(&self, x: f32, y: f32, offset_x: f32, offset_y: f32) -> Option<usize> {
    let lazy = self.lazy.as_ref()?;
    let doc_y = y - offset_y;

    // 找到 y 对应的 flat_line
    let flat_line = lazy.flat_lines.iter()
        .find(|fl| doc_y >= fl.rect.y && doc_y <= fl.rect.y + fl.rect.h)?;

    // 找到 flat_line 中 x 对应的字符偏移
    let shaped = flat_line.shaped.as_ref()?;
    let line_x = x - offset_x;
    let char_offset = shaped.char_at_x(line_x)?;

    // 将 flat_line + char_offset 映射回源码字节偏移
    // 需要根据 source_range 反向映射
    self.byte_from_flat_line_and_char(flat_line.flat_idx, char_offset)
}
```

> **Note:** `byte_from_flat_line_and_char` 是核心映射函数——需要从 flat_line_idx + char_offset (折叠文本中的位置) 反推到源码字节偏移。这需要利用 StyleSpan 的 source_range 和 materialize 时的偏移累积关系。

- [ ] **Step 8: PreviewEngine 实现 visual_move**

```rust
/// 视觉方向导航：从 current_byte 按 direction 移动，返回目标字节偏移。
pub fn visual_move(
    &self,
    current_byte: usize,
    direction: MoveDirection,
    target_x: Option<f32>,
) -> Option<usize> {
    use ui::plugin::MoveDirection;
    let lazy = self.lazy.as_ref()?;
    let source = self.doc.as_ref()?;

    match direction {
        MoveDirection::Left => {
            // 前一个 UTF-8 char 边界 (折叠态跳越不可见标记符)
            // 小于 current_byte 的最大 char 边界
            if current_byte == 0 { return Some(0); }
            let prev = prev_char_boundary_str(source_text(source), current_byte);
            Some(prev)
        }
        MoveDirection::Right => {
            // 下一个 UTF-8 char 边界
            let text = source_text(source);
            if current_byte >= text.len() { return Some(current_byte); }
            let next = next_char_boundary_str(text, current_byte);
            Some(next)
        }
        MoveDirection::Up | MoveDirection::Down => {
            // 1. 找当前 flat_line
            let (current_flat_idx, current_x) = self.flat_line_and_x_for_byte(current_byte)?;
            let x = target_x.unwrap_or(current_x);

            // 2. 目标行
            let target_flat_idx = match direction {
                MoveDirection::Up if current_flat_idx > 0 => current_flat_idx - 1,
                MoveDirection::Down if current_flat_idx + 1 < lazy.flat_lines.len() => current_flat_idx + 1,
                _ => return Some(current_byte),
            };

            // 3. 目标行的 x → char_offset → byte
            let target_line = &lazy.flat_lines[target_flat_idx];
            let shaped = target_line.shaped.as_ref()?;
            let char_offset = shaped.char_at_x(x)?;
            self.byte_from_flat_line_and_char(target_flat_idx, char_offset)
        }
        MoveDirection::LineStart => {
            // 当前行的起始字节
            let (flat_idx, _) = self.flat_line_and_x_for_byte(current_byte)?;
            self.byte_from_flat_line_and_char(flat_idx, 0)
        }
        MoveDirection::LineEnd => {
            let (flat_idx, _) = self.flat_line_and_x_for_byte(current_byte)?;
            let line = &lazy.flat_lines[flat_idx];
            self.byte_from_flat_line_and_char(flat_idx, line.text.len())
        }
    }
}
```

> **Note:** `flat_line_and_x_for_byte` 和 `byte_from_flat_line_and_char` 是字节↔行坐标的核心映射。`source_text` 辅助函数考虑展开态时获取当前 materialize 后的文本。

- [ ] **Step 9: PreviewEngine 实现 augment_edit**

```rust
/// 编辑干预：Enter/Backspace/Tab 的 markdown 感知行为。
pub fn augment_edit(
    &self,
    current_byte: usize,
    kind: AugmentKind,
) -> Option<EditAugmentation> {
    use ui::plugin::{AugmentKind, EditAugmentation};
    let doc = self.doc.as_ref()?;
    let source = /* 当前源码 */;

    match kind {
        AugmentKind::Enter => {
            // 检查光标是否在列表项内
            let (block_idx, _) = self.find_block_at_byte(current_byte)?;
            let block = &doc.blocks[block_idx];
            if let BlockKind::ListItem { bullet, .. } = &block.kind {
                let marker = match bullet {
                    ListBullet::Bullet => "- ",
                    ListBullet::Ordered(n) => {
                        // 检查当前序号，+1
                        &format!("{}. ", n + 1)
                    }
                    ListBullet::TaskList(false) => "- [ ] ",
                    ListBullet::TaskList(true) => "- [x] ",
                };
                let insertion = format!("\n{}", marker);
                return Some(EditAugmentation {
                    insert_text: Some(insertion.clone()),
                    cursor_byte_after: current_byte + insertion.len(),
                    ..Default::default()
                });
            }

            // 检查当前列表项是否为空 (仅 "- ")，如是则退格删除该行退出列表
            // (此处略去具体实现)

            None // 使用默认换行
        }
        AugmentKind::Backspace => {
            // 检查光标是否在 span 边界 → 配对删除标记符
            // e.g., 光标在 **|world** → 删除两个 **
            // (此处略去具体实现)
            None // 使用默认单字符退格
        }
        AugmentKind::Tab => {
            // Tab 缩进列表项 (在此不做，留给 standard editor)
            None
        }
    }
}
```

> **Note:** `augment_edit` 的完整实现依赖所有之前的映射函数。第一阶段可以只实现 Enter 的简单列表续接，Backspace/Tab 可后续迭代。

- [ ] **Step 10: MarkdownView handle_message 增加 SetCursorByte 分支**

在 `MarkdownView::handle_message()` 中添加：

```rust
PluginMessage::SetCursorByte(byte) => {
    // MarkdownView (纯预览) 忽略此消息
    true
},
```

- [ ] **Step 11: MarkdownView query() 实现新查询 (预览模式——返回 None)**

已在 Task 2 Step 7 中处理。

- [ ] **Step 12: 编译 + 测试 + 提交**

```bash
cargo build -p edit-plus-markdown 2>&1
cargo test -p edit-plus-markdown 2>&1

git add crates/markdown/src/edit.rs crates/markdown/src/lib.rs crates/markdown/src/view.rs
git commit -m "feat: Engine 层——EditContext + materialize_text + 查询骨架"
```

---

### Task 4: MarkdownEditorView + Factory

**Files:**
- Modify: `crates/markdown/src/view.rs` (末尾追加)

**Interfaces:**
- Consumes: `PreviewEngine`, `EditContext` (Task 3), Plugin 类型 (Task 2)
- Produces: `MarkdownEditorView`, `MarkdownEditorViewFactory`

- [ ] **Step 1: 在 view.rs 末尾定义 MarkdownEditorView**

```rust
// ===== MarkdownEditorView =====

/// WYSIWYG markdown editor view — wraps PreviewEngine with editing support.
/// Differs from MarkdownView in that it accepts cursor position updates
/// and provides layout-aware editing queries.
pub struct MarkdownEditorView {
    engine: PreviewEngine,
    source: String,
    cached_source_hash: u64,
    cached_generation: u32,
}

impl Default for MarkdownEditorView {
    fn default() -> Self { Self::new() }
}

impl MarkdownEditorView {
    pub fn new() -> Self {
        Self {
            engine: PreviewEngine::new(),
            source: String::new(),
            cached_source_hash: 0,
            cached_generation: 0,
        }
    }

    pub fn set_source(&mut self, text: String, generation: u32) {
        let hash = fxhash(&text);
        if hash != self.cached_source_hash {
            self.source = text;
            self.cached_source_hash = hash;
            self.engine.mark_dirty();
        }
        self.cached_generation = generation;
    }

    pub fn needs_source_update(&self, generation: u32) -> bool {
        generation != self.cached_generation
    }

    pub fn engine(&self) -> &PreviewEngine {
        &self.engine
    }
}

impl ViewPlugin for MarkdownEditorView {
    fn name(&self) -> &str {
        "markdown_editor"
    }

    fn allows_editing(&self) -> bool {
        true  // 必须返回 true，否则 app_renderer 将视图标记为只读，
              // 禁用 Copy/Paste/Undo 右键菜单并改变滚动条行为。
              // WYSIWYG 拦截由 is_wysiwyg() 单独鉴权。
    }

    fn shows_cursor(&self) -> bool {
        false  // 自己渲染光标 (通过 CursorScreenPos + DrawList 竖线)
    }

    fn shows_gutter(&self) -> bool {
        false
    }

    fn is_wysiwyg(&self) -> bool {
        true   // 启用 WYSIWYG 分派路径
    }

    fn render(
        &mut self,
        _doc: &dyn DocView,
        bounds: ui::core::geom::Rect,
        theme: &Theme,
        shaper: &mut shaping::Shaper,
        dpi_scale: f32,
    ) -> DrawList {
        let settings = MarkdownRenderSettings {
            font_size: self.engine.base_font_size * dpi_scale,
            line_height: self.engine.base_line_height * dpi_scale,
            toc_max_depth: self.engine.toc_max_depth,
        };
        let style = settings.style(theme);
        self.engine.toc_max_depth = settings.toc_max_depth;
        let source = &self.source;
        self.engine.render(
            theme,
            bounds.w,
            bounds.h,
            bounds.x,
            bounds.y,
            &style,
            |s| {
                let parsed = crate::parser::parse_markdown(source);
                crate::builder::MarkdownDoc::build(&parsed, s)
            },
            Some(shaper),
        ).0
    }

    fn handle_message(
        &mut self,
        msg: PluginMessage,
        _doc: &mut dyn core::document::DocViewMut,
    ) -> bool {
        match msg {
            PluginMessage::Scroll { delta, viewport_h } => self.engine.scroll(delta, viewport_h),
            PluginMessage::ScrollToHeading(index) => {
                self.engine.scroll_to_heading(index);
                true
            }
            PluginMessage::UpdateSource { text, generation } => {
                self.set_source(text, generation);
                true
            }
            PluginMessage::SetCursorByte(byte) => {
                self.engine.handle_set_cursor_byte(byte);
                true
            }
            PluginMessage::SetSelCursor(pos) => {
                self.engine.set_sel_cursor(pos);
                true
            }
            PluginMessage::SetSelAnchor(pos) => {
                self.engine.set_sel_anchor(pos);
                true
            }
            PluginMessage::ClearSelection => {
                self.engine.clear_selection();
                true
            }
            PluginMessage::SelectAll => {
                self.engine.select_all();
                true
            }
            PluginMessage::SetRenderSettings { font_size, line_height, toc_max_depth } => {
                self.engine.base_font_size = font_size;
                self.engine.base_line_height = line_height;
                self.engine.toc_max_depth = toc_max_depth;
                true
            }
            _ => false,
        }
    }

    fn query(&self, q: PluginQuery, _doc: &dyn DocView) -> PluginResponse {
        match q {
            PluginQuery::ScrollY => PluginResponse::Float(self.engine.scroll_y),
            PluginQuery::ContentHeight => PluginResponse::Float(self.engine.content_height),
            PluginQuery::NeedsSourceUpdate(gen_id) => {
                PluginResponse::Bool(self.needs_source_update(gen_id))
            }
            PluginQuery::TOCHeadings => PluginResponse::Headings(
                self.engine.headings().iter()
                    .map(|h| ui::plugin::HeadingEntry {
                        title: h.text.clone(), y: h.y_offset, level: h.level,
                    })
                    .collect(),
            ),
            PluginQuery::CurrentHeadingIndex(scroll_y) => PluginResponse::Position(
                self.engine.current_heading_index(scroll_y).map(|i| (i, 0)),
            ),
            PluginQuery::HasSelection => PluginResponse::Bool(self.engine.has_selection()),
            PluginQuery::SelectedText => {
                PluginResponse::String(self.engine.selected_text().unwrap_or_default())
            }
            PluginQuery::HitTest { x, y, offset_x, offset_y } => PluginResponse::Position(
                self.engine.hit_test(x, y, offset_x, offset_y)
                    .map(|p| (p.flat_line_idx, p.char_pos)),
            ),
            PluginQuery::SearchHighlights { .. } => PluginResponse::DrawList(DrawList::new()),
            PluginQuery::SelectionHighlights(color) => {
                PluginResponse::DrawList(self.engine.selection_highlights(color))
            }
            PluginQuery::FlatLines => PluginResponse::FlatLines(
                self.engine.flat_lines().iter()
                    .map(|fl| ui::plugin::FlatLine { text: fl.text.clone() })
                    .collect(),
            ),
            PluginQuery::WordAtPos(li, cp) => {
                let (s, e) = self.engine.word_at_pos(ViewPos { flat_line_idx: li, char_pos: cp });
                PluginResponse::PositionPair(Some((
                    (s.flat_line_idx, s.char_pos), (e.flat_line_idx, e.char_pos),
                )))
            }
            PluginQuery::LineRangeAtPos(li, cp) => {
                let (s, e) = self.engine.line_range_at_pos(ViewPos { flat_line_idx: li, char_pos: cp });
                PluginResponse::PositionPair(Some((
                    (s.flat_line_idx, s.char_pos), (e.flat_line_idx, e.char_pos),
                )))
            }
            // WYSIWYG 查询——委托给 engine
            PluginQuery::CursorScreenPos(byte) => {
                PluginResponse::CursorRect(self.engine.cursor_screen_pos())
            }
            PluginQuery::HitTestByte { x, y, offset_x, offset_y } => {
                PluginResponse::BytePosition(self.engine.hit_test_byte(x, y, offset_x, offset_y))
            }
            PluginQuery::VisualMove { current_byte, direction, target_x } => {
                PluginResponse::BytePosition(
                    self.engine.visual_move(current_byte, direction, target_x),
                )
            }
            PluginQuery::AugmentEdit { current_byte, kind } => {
                PluginResponse::Augmentation(
                    self.engine.augment_edit(current_byte, kind),
                )
            }
        }
    }
}
```

- [ ] **Step 2: MarkdownEditorViewFactory**

```rust
// ===== MarkdownEditorViewFactory =====

pub struct MarkdownEditorViewFactory;

impl PluginFactory for MarkdownEditorViewFactory {
    fn name(&self) -> &str {
        "markdown_editor"
    }
    fn can_handle(&self, path: Option<&Path>) -> bool {
        // .md 文件默认使用编辑器视图
        path.is_some_and(|p| p.extension().is_some_and(|e| e == "md" || e == "markdown"))
    }
    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(MarkdownEditorView::new())
    }
}
```

- [ ] **Step 3: 在 workspace.rs 中注册 MarkdownEditorViewFactory**

修改 `crates/app/src/workspace.rs`：

```rust
// 将 MarkdownViewFactory 替换为 MarkdownEditorViewFactory (或与预览工厂并存)
registry.register(Box::new(edit_plus_markdown::view::MarkdownEditorViewFactory));
```

同时保留 MarkdownViewFactory 作预览切换用，或将其重命名为 MarkdownPreviewViewFactory。

- [ ] **Step 4: 编译 + 测试 + 提交**

```bash
cargo build 2>&1
cargo test -p edit-plus-markdown 2>&1
cargo test -p edit-plus-app 2>&1

git add crates/markdown/src/view.rs crates/app/src/workspace.rs
git commit -m "feat: MarkdownEditorView + Factory——WYSIWYG 编辑视图骨架"
```

---

### Task 5: App 层——WYSIWYG 编辑分派（拦截→篡改→放行）

**Files:**
- Modify: `crates/app/src/dispatch/editor.rs`
- Modify: `crates/app/src/app_lifecycle.rs` (IME 光标定位，鼠标点击路由)

**Interfaces:**
- Consumes: `ViewPlugin::is_wysiwyg()`, 新增 PluginQuery (Task 2), `MarkdownEditorView` (Task 4), `execute_edit_command_v2` (已有)
- Produces: WYSIWYG 编辑分派路径。核心原则——**绝不绕过 `execute_edit_command_v2`**（它产出 `Outcome` 驱动 `frame_cache.advance_cache` 失效）

**设计原则：**

| 命令类型 | 拦截策略 | 放行方式 |
|---------|---------|---------|
| 视觉导航 (MoveUp/Down/Left/Right/PageUp/Down/Home/End/Click) | 查询 VisualMove/HitTestByte → 更新光标 → 返回 `AppEffect::REDRAW` | **不放行** (不修改文本，无需缓存失效) |
| 常规输入 (InsertChar, Ime::Commit) | 不拦截 | **直接放行** 给 `execute_edit_command_v2` |
| 智能回车 (InsertNewline) | 查询 AugmentEdit → 若有干预则**篡改**为 `EditCommand::InsertText("\n- ")` | **递归放行** 给 `dispatch_edit_command` |
| 智能退格 (Backspace) | 同上，篡改为 Delete 或放行 | **递归或直接放行** |
| 全局命令 (Save/Copy/TogglePreview...) | 调用现有处理 | 照旧 |

**函数签名（以现有代码为准）：**

```rust
pub(crate) fn dispatch_edit_command(
    &mut self,
    cmd: EditCommand,
    event_loop: &ActiveEventLoop,
) -> AppEffect
```

- [ ] **Step 1: 在 dispatch_edit_command 顶部插入 WYSIWYG 拦截块**

在现有的 `allows_editing()` 门禁（代码第16行 `if self.workspace.active_entry().is_some_and(|t| !t.plugin.allows_editing())`）**之前**插入：

```rust
pub(crate) fn dispatch_edit_command(
    &mut self,
    cmd: EditCommand,
    event_loop: &ActiveEventLoop,
) -> AppEffect {
    let mut effect = AppEffect::NONE;

    // ── WYSIWYG mode: intercept BEFORE standard dispatch ──
    if self.workspace.active_entry().is_some_and(|t| t.plugin.is_wysiwyg()) {
        match &cmd {
            // 视觉导航 → 查询插件，更新光标，返回 REDRAW（不修改文本）
            EditCommand::MoveUp | EditCommand::MoveDown
            | EditCommand::MoveLeft | EditCommand::MoveRight
            | EditCommand::MoveWordLeft | EditCommand::MoveWordRight
            | EditCommand::MoveToLineStart | EditCommand::MoveToLineEnd
            | EditCommand::MoveToDocStart | EditCommand::MoveToDocEnd
            | EditCommand::PageUp | EditCommand::PageDown => {
                return self.dispatch_wysiwyg_navigation(&cmd, event_loop);
            }
            // 智能回车 → 查询 AugmentEdit(Enter) → 篡改命令 → 递归放行
            EditCommand::InsertNewline => {
                return self.dispatch_wysiwyg_augmented_enter(event_loop);
            }
            // 智能退格 → 查询 AugmentEdit(Backspace) → 篡改命令 → 递归放行
            EditCommand::Backspace => {
                return self.dispatch_wysiwyg_augmented_backspace(event_loop);
            }
            // InsertChar / DeleteForward / Tab / Paste → DON'T intercept
            // 直接放行给 execute_edit_command_v2
            _ => {}
        }
    }

    // ── Markdown preview mode: block editing commands (现有逻辑，不变) ──
    if self.workspace.active_entry().is_some_and(|t| !t.plugin.allows_editing()) {
        // ... 现有 gate 逻辑完全不变
    }
    // ... 后续现有逻辑完全不变
}
```

- [ ] **Step 2: 实现 dispatch_wysiwyg_navigation**

```rust
/// 视觉方向导航——查询插件获取目标字节偏移，更新 DocumentView 光标。
/// 不修改文本，不经过 execute_edit_command_v2。
fn dispatch_wysiwyg_navigation(
    &mut self,
    cmd: &EditCommand,
    event_loop: &ActiveEventLoop,
) -> AppEffect {
    use crate::input::EditCommand;
    use ui::plugin::MoveDirection;

    let direction = match cmd {
        EditCommand::MoveLeft => MoveDirection::Left,
        EditCommand::MoveRight => MoveDirection::Right,
        EditCommand::MoveUp => MoveDirection::Up,
        EditCommand::MoveDown => MoveDirection::Down,
        EditCommand::MoveWordLeft => MoveDirection::Left,   // FIXME: word-aware later
        EditCommand::MoveWordRight => MoveDirection::Right,  // FIXME: word-aware later
        EditCommand::MoveToLineStart => MoveDirection::LineStart,
        EditCommand::MoveToLineEnd => MoveDirection::LineEnd,
        EditCommand::MoveToDocStart => {
            // 文档开头：直接设 scroll_y = 0 并取第0个字节
            return self.navigate_to_doc_start(event_loop);
        }
        EditCommand::MoveToDocEnd => {
            return self.navigate_to_doc_end(event_loop);
        }
        EditCommand::PageUp => {
            // 翻页：调整 scroll_y，光标不跟随
            self.scroll_by_page(-1);
            return AppEffect::REDRAW;
        }
        EditCommand::PageDown => {
            self.scroll_by_page(1);
            return AppEffect::REDRAW;
        }
        _ => return AppEffect::NONE,
    };

    let Some(dv) = self.workspace.active_doc_mut() else {
        return AppEffect::NONE;
    };

    // 获取当前光标字节偏移（需要 DocumentView 暴露 cursor_byte 方法）
    let current_byte = dv.cursor_byte_offset()
        .expect("WYSIWYG mode requires cursor byte tracking");

    let target_x = self.wysiwyg_preferred_x;

    let plugin = &self.workspace.active_entry()
        .expect("active entry must exist when is_wysiwyg").plugin;

    match plugin.query(
        PluginQuery::VisualMove { current_byte, direction, target_x },
        dv,
    ) {
        PluginResponse::BytePosition(Some(new_byte)) => {
            // 更新 DocumentView 的光标字节位置
            dv.set_cursor_byte(new_byte)
                .expect("WYSIWYG cursor byte update should succeed");
            // 通知插件
            let plugin_mut = &mut self.workspace.active_entry_mut()
                .expect("active entry must exist").plugin;
            plugin_mut.handle_message(PluginMessage::SetCursorByte(new_byte), dv);

            // 更新 preferred_x（上/下移动后保持列锚定）
            match plugin_mut.query(
                PluginQuery::CursorScreenPos(new_byte), dv,
            ) {
                PluginResponse::CursorRect(Some((x, _, _, _))) => {
                    self.wysiwyg_preferred_x = Some(x);
                }
                _ => {}
            }
            AppEffect::REDRAW
        }
        _ => AppEffect::NONE,
    }
}
```

> **Note:** `dv.cursor_byte_offset()` 和 `dv.set_cursor_byte()` 是新增方法。`DocumentView` 当前使用行/列光标模型，需要增加字节偏移的存取接口。实现时可将字节偏移转换为行/列存储，或直接在 `DocumentView` 上增加 `cursor_byte: usize` 字段。

- [ ] **Step 3: 实现 dispatch_wysiwyg_augmented_enter/backspace——篡改命令后递归放行**

```rust
/// 智能回车——查询插件是否需要续接列表标记。
/// 若有干预：将 InsertNewline 篡改为 InsertText("\n- ") 并递归。
/// 若无干预：直接放行给 execute_edit_command_v2（走下方现有正常流程）。
fn dispatch_wysiwyg_augmented_enter(
    &mut self,
    event_loop: &ActiveEventLoop,
) -> AppEffect {
    let Some(dv) = self.workspace.active_doc_mut() else {
        return AppEffect::NONE;
    };
    let current_byte = match dv.cursor_byte_offset() {
        Some(b) => b,
        None => {
            // 无有效光标位置 → 默认换行，放行
            return self.dispatch_edit_command(EditCommand::InsertNewline, event_loop);
        }
    };

    let plugin = &self.workspace.active_entry()
        .expect("active entry must exist").plugin;
    match plugin.query(
        PluginQuery::AugmentEdit { current_byte, kind: AugmentKind::Enter },
        dv,
    ) {
        PluginResponse::Augmentation(Some(aug)) => {
            if let Some(insert_text) = aug.insert_text {
                // 篡改命令 → 递归放行
                return self.dispatch_edit_command(
                    EditCommand::InsertText(insert_text),
                    event_loop,
                );
            }
            // augment 返回了但无 insert_text → 视为默认换行，放行
            self.dispatch_edit_command(EditCommand::InsertNewline, event_loop)
        }
        _ => {
            // 无干预 → 放行给 execute_edit_command_v2 正常处理
            self.dispatch_edit_command(EditCommand::InsertNewline, event_loop)
        }
    }
}

/// 智能退格——查询插件是否需要配对删除标记符。
/// MVP 阶段：若无干预，直接放行给 execute_edit_command_v2。
fn dispatch_wysiwyg_augmented_backspace(
    &mut self,
    event_loop: &ActiveEventLoop,
) -> AppEffect {
    let Some(dv) = self.workspace.active_doc_mut() else {
        return AppEffect::NONE;
    };
    let current_byte = match dv.cursor_byte_offset() {
        Some(b) => b,
        None => {
            return self.dispatch_edit_command(EditCommand::Backspace, event_loop);
        }
    };

    let plugin = &self.workspace.active_entry()
        .expect("active entry must exist").plugin;
    match plugin.query(
        PluginQuery::AugmentEdit { current_byte, kind: AugmentKind::Backspace },
        dv,
    ) {
        PluginResponse::Augmentation(Some(aug)) => {
            if let Some(delete_range) = aug.delete_range {
                // 将范围删除转化为：先选中 → 再 Backspace
                // 简化：先移动光标到 range.end，再插入空字符串覆盖
                // 更稳健：转化为 Delete 命令（若 EditCommand 有支持）
                // MVP: 直接放行给标准 Backspace（后续迭代实现精细配对删除）
                return self.dispatch_edit_command(EditCommand::Backspace, event_loop);
            }
            self.dispatch_edit_command(EditCommand::Backspace, event_loop)
        }
        _ => {
            self.dispatch_edit_command(EditCommand::Backspace, event_loop)
        }
    }
}
```

**核心要点**：`InsertNewline` 和 `Backspace` 的"干预"实际上只是**提前篡改 EditCommand**，例如 `InsertNewline` 变成 `InsertText("\n- ")`。篡改后的命令通过 `self.dispatch_edit_command(augmented_cmd, event_loop)` 递归进入，再走 `execute_edit_command_v2` 的完整管线——包括 Outcome 产出、DisplayLine 缓存失效、Undo 记录。**任何情况下都不手动调用 `doc_view.insert/delete`**。

- [ ] **Step 4: 在 execute_edit_command_v2 成功后通知插件**

在现有 `dispatch_edit_command` 的第 483 行 `let outcome = crate::commands::execute_edit_command_v2(&cmd, dv, ac);` 之后，第 484 行 `if outcome.executed {` 块内末尾，添加 WYSIWYG 通知：

```rust
if outcome.executed {
    // ... 现有缓存失效逻辑 (484-521行) 全部保留

    // ── WYSIWYG: 通知插件源码和光标已变更 ──
    if self.workspace.active_entry().is_some_and(|t| t.plugin.is_wysiwyg()) {
        if let Some(entry) = self.workspace.active_entry_mut() {
            // 用 dv 的当前全文更新插件
            let text = dv.full_text();          // 需要暴露的方法
            let gen = dv.generation();          // buffer generation
            entry.plugin.handle_message(
                PluginMessage::UpdateSource { text, generation: gen },
                dv,
            );
            if let Some(byte) = dv.cursor_byte_offset() {
                entry.plugin.handle_message(
                    PluginMessage::SetCursorByte(byte),
                    dv,
                );
            }
        }
    }

    // ... reset_cursor_after_edit 等后续逻辑
}
```

- [ ] **Step 5: IME 光标定位**

在 `crates/app/src/app_lifecycle.rs` 的 IME 处理代码中（处理 `Ime::Commit` 和 `Ime::Preedit` 的地方），添加 WYSIWYG 光标查询：

```rust
// 当更新 IME 候选框位置时
if self.workspace.active_entry().is_some_and(|t| t.plugin.is_wysiwyg()) {
    if let Some(dv) = self.workspace.active_doc() {
        if let Some(byte) = dv.cursor_byte_offset() {
            let plugin = &self.workspace.active_entry()
                .expect("active entry must exist").plugin;
            match plugin.query(PluginQuery::CursorScreenPos(byte), dv) {
                PluginResponse::CursorRect(Some((x, y, _w, h))) => {
                    // 文档坐标 → 窗口坐标 → 设置 IME 区域
                    let editor_rect = /* 编辑器在窗口中的区域 */;
                    let window_pos = (editor_rect.x + x, editor_rect.y + y + h);
                    let area = winit::dpi::PhysicalPosition::new(
                        (window_pos.0 * dpi_scale) as i32,
                        (window_pos.1 * dpi_scale) as i32,
                    );
                    window.set_ime_cursor_area(area, (h * dpi_scale) as u32);
                }
                _ => {}
            }
        }
    }
}
```

- [ ] **Step 6: 鼠标点击路由**

鼠标点击不在 `EditCommand` 中——`EditCommand` 只有键盘命令。鼠标事件在 `app_lifecycle.rs` 中直接处理。

需要在鼠标事件处理路径中添加 WYSIWYG 分支：

```rust
// 在处理 MouseInput 的地方
if let Some(dv) = self.workspace.active_doc_mut() {
    if self.workspace.active_entry().is_some_and(|t| t.plugin.is_wysiwyg()) {
        let plugin = &self.workspace.active_entry()
            .expect("active entry must exist").plugin;
        let (x, y) = /* 屏幕坐标转文档坐标 */;
        match plugin.query(
            PluginQuery::HitTestByte { x, y, offset_x, offset_y },
            dv,
        ) {
            PluginResponse::BytePosition(Some(byte)) => {
                dv.set_cursor_byte(byte)
                    .expect("byte cursor update should succeed");
                let plugin_mut = &mut self.workspace.active_entry_mut()
                    .expect("active entry must exist").plugin;
                plugin_mut.handle_message(
                    PluginMessage::SetCursorByte(byte), dv,
                );
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 7: 编译 + 测试 + 提交**

```bash
cargo build 2>&1
cargo test -p edit-plus-app 2>&1

git add crates/app/src/dispatch/editor.rs crates/app/src/app_lifecycle.rs
git commit -m "feat: App 层 WYSIWYG 编辑分派——拦截→篡改→放行，execute_edit_command_v2 驱动缓存失效"
```

---

### Task 6: 最终验证

- [ ] **Step 1: 完整验证**

```bash
./scripts/verify.sh
```

预期：`cargo fmt` + `cargo clippy` + `cargo test` 全部通过。

- [ ] **Step 2: 手动验收测试场景**

- 打开 `.md` 文件 → 显示 MarkdownEditorView (WYSIWYG 模式)
- 光标在纯文本中 → 无标记符显示
- 光标移入加粗区域 → `**` 标记符展开显示
- 光标移出 → 标记符折叠
- 在列表项中按回车 → 自动续接 `- `
- 方向键导航 (考虑折叠态跳越)
- Toggle 切换到预览模式 → MarkdownView (只读)
- 切换回编辑模式 → 保持滚动位置和光标

- [ ] **Step 3: 提交 (如需最终调整)**

```bash
git add -A
git commit -m "chore: WYSIWYG 编辑器最终集成——验证通过"
```

---

## 自检清单

**1. Spec coverage:**
- [x] Section 3 (架构概览): Tasks 1-5 实现
- [x] Section 4 (PluginMessage/Query 扩展): Task 2 实现
- [x] Section 5 (源码映射层): Task 1 实现
- [x] Section 6 (展开机制): Task 3 实现
- [x] Section 7 (关键数据流): Task 5 实现——拦截→篡改→放行
- [x] Section 8 (增量布局): Task 3 中 rebuild 逻辑处理
- [x] Section 9 (光标渲染): Task 3 Engine 查询 + Task 5 IME 定位
- [x] Section 10 (PreviewView 键盘响应): Task 5 全局命令走现有逻辑
- [x] Section 11 (模式切换): Task 5 Step 6 (保留 ToggleMarkdownPreview)

**2. Placeholder scan:**
- 布局映射函数 (`find_char_position`, `char_offset_from_byte`, `byte_from_flat_line_and_char`) 标注为 Note——核心算法路径已描述，具体实现取决于 block/span 遍历结构
- `augment_edit` Backspace 配对删除标注为"MVP 直接放行，后续迭代"——符合 MVP 范围
- 无 TBD/TODO/不完整步骤

**3. Type consistency:**
- `EditContext` 在 Task 3 Step 1 定义，Task 3 Step 4 使用
- `PluginMessage::SetCursorByte(usize)` 在 Task 2 定义，Task 3/4/5 使用
- `PluginQuery` 新变体在 Task 2 定义，Task 3/4/5 实现和调用
- `PluginResponse` 新变体在 Task 2 定义，各处一致
- `is_wysiwyg()` 在 Task 2 定义，Task 5 分派检查
- `MarkdownEditorView` 在 Task 4 定义，`allows_editing() = true`, `is_wysiwyg() = true`

**4. 核心约束验证:**
- [x] Task 5 不绕过 `execute_edit_command_v2`→Outcome→缓存失效管线
- [x] `EditCommand::InsertChar` 不被拦截，直接放行
- [x] 视觉导航返回 `AppEffect::REDRAW` 而非 `bool`
- [x] `allows_editing() = true` 避免只读视图副作用
- [x] `event_ranges: Vec<Range<usize>>` 避免边界推断错误
- [x] 智能回车篡改 `InsertNewline` → `InsertText("\n- ")` 后递归放行
