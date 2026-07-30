# Novel 路径零拷贝 AST 设计

## 1. 目标

消除 Novel 模式（txt 小说文件）打开时 `build_from_novel_doc` 中的全量 `String` 分配。
10 万行（~5MB）文本的 AST 构建从 ~38ms 降至 ~2ms，堆分配从 5MB 降至 ~50KB。

## 2. 现状与瓶颈

当前 Novel 渲染数据流：

```
DocView (TextBuffer/GapBuffer)
  │
  ▼ build_from_novel_doc(doc)
  │   每行 doc.doc_line_bytes(i) → String::from_utf8_lossy → into_owned()  ← ~38ms, 10万次分配
  ▼
MarkdownDoc { blocks: Vec<BlockNode> }
  │  └─ BlockNode.text_lines: Vec<String>  ← 全量文本副本
  ▼ LazyLayout::from_doc()
  │  └─ collect_text_lines(block)  ← 再次 clone
  ▼
LazyLayout (全量估计布局)
  │
  ▼ ensure_precise_range()  ← 仅可见块做 HarfBuzz 塑形
  ▼
屏幕像素
```

核心浪费：`text_lines: Vec<String>` 持有全量文本的独立副本，而底层 `TextBuffer` 已有完整数据。首屏仅需 ~60 行。

## 3. 改造后数据流

```
DocView (TextBuffer/GapBuffer) ────────────────────────────┐
  │                                                        │
  ▼ build_from_novel_doc(doc)     ← 仅存字节范围，零分配    │
MarkdownDoc {                                               │
  blocks: Vec<BlockNode>                                    │
    text_lines: vec![]              ← 空                     │
    line_src_ranges: Vec<Range<usize>>  ← 每行字节范围      │
    source_range: Range<usize>      ← 块级字节范围           │
}                                                           │
  ▼ LazyLayout::from_doc_lazy()   ← 字节长度估算行数        │
  │                                                            │
  ▼ ensure_precise_range(source) ← 可见块按需取文本 ─────────┘
  │    doc.doc_text_in_range(line_range)  ← 仅 ~60 行调用
  ▼
屏幕像素
```

## 4. 逐层改动

### 4.1 BlockNode 扩展 (`builder.rs`)

新增字段：

```rust
pub struct BlockNode {
    // ... 现有字段不变 ...
    pub text_lines: Vec<String>,            // markdown: 有值; novel: 空
    pub source_range: Range<usize>,          // 已有，novel 继续使用
    // 新增:
    pub line_src_ranges: Vec<Range<usize>>, // novel 每行字节范围; markdown: 空
}
```

### 4.2 build_from_novel_doc 零分配 (`builder.rs`)

当前每行 `text.into_owned()` 产生 `String` 分配。改为只记录字节范围：

```rust
// 当前:
para_lines.push(text.into_owned());  // String 分配

// 改造后: 记录行字节范围
para_line_ranges.push(line_start_byte..line_end_byte);
```

`BlockNode` 产出：
```rust
BlockNode {
    kind: BlockKind::Paragraph,
    text_lines: vec![],                      // 不分配
    text_styles: vec![vec![]; line_count],   // novel 无样式
    source_range: para_start..para_end,
    line_src_ranges: para_line_ranges,       // 字节范围
    ..
}
```

### 4.3 字节估算 (`layout.rs`)

现有 `estimate_line_count(text, max_w, font_size)` 依赖 `text.chars().count()`。
新增字节版估算，无需文本：

```rust
fn estimate_line_count_from_byte_len(byte_len: usize, max_w: f32, font_size: f32) -> usize {
    if byte_len == 0 { return 1; }
    let char_w = font_size * 0.55;
    let chars_per_line = (max_w / char_w).max(1.0) as usize;
    // 保守估计: 假设全部 ASCII (byte_len ≈ char_count)
    // 对 CJK (3 bytes/char) 会高估行数，precision pass 通过 y_delta 修正
    byte_len.div_ceil(chars_per_line)
}
```

新增 `from_doc_lazy` 构造器。遍历 `BlockNode` 时：
- `text_lines` 非空 → 走现有 `collect_text_lines` 路径
- `text_lines` 为空且有 `line_src_ranges` → 用字节估算

### 4.4 Precision 按需取文本 (`layout.rs`)

`ensure_precise_range` 和 `precise_block_at` 新增 `source: &dyn DocView` 参数。

`layout_text_block` 中 shaper 路径需要真实文本时：

```rust
let raw_lines: Vec<String> = if block.text_lines.is_empty() && !block.line_src_ranges.is_empty() {
    block.line_src_ranges.iter().map(|r| {
        source.doc_text_in_range(r.clone()).into_owned()
    }).collect()
} else {
    collect_text_lines(block)
};
```

`doc_text_in_range` 在行不跨 Gap Buffer 时返回 `Cow::Borrowed`（零拷贝）；跨 Gap 时才 `into_owned()`。

### 4.5 PreviewEngine 透传 (`view.rs`)

`render()` 签名增加 `source: Option<&dyn DocView>`：

```rust
pub fn render(
    &mut self,
    // ... 现有参数 ...
    source: Option<&dyn DocView>,  // Novel: Some(doc), Markdown: None
) -> (DrawList, bool)
```

`rebuild_layout` 内部判断：`source.is_some()` 且 blocks 有 `line_src_ranges` → 走 lazy 路径。

### 4.6 View 层适配 (`view.rs`)

- `NovelView::render` → 传 `Some(doc)` 
- `MarkdownView::render` / `MarkdownEditorView::render` → 传 `None`，行为完全不变

## 5. 影响范围

| 文件 | 改动量 | 描述 |
|------|--------|------|
| `markdown/src/builder.rs` | ~50 行 | BlockNode 加字段; `build_from_novel_doc` 零分配 |
| `markdown/src/layout.rs` | ~80 行 | `from_doc_lazy`、字节估算、precision 分支 |
| `markdown/src/view.rs` | ~30 行 | `render`/`rebuild` 透传 `Option<&dyn DocView>` |
| `markdown/src/view.rs` (NovelView) | 5 行 | 传 `Some(doc)` |
| `markdown/src/view.rs` (MarkdownView) | 2 行 | 传 `None` |

**不动**：WYSIWYG 编辑路径、Markdown 解析/布局管线、所有现有测试。

## 6. 收益预估

| 指标 | 改造前 | 改造后 |
|------|--------|--------|
| `build_from_novel_doc` 耗时 | ~38ms (10万次 String::into_owned) | ~2ms (10万次 Range 记录) |
| AST 堆分配 | ~5MB (全量文本副本) | ~50KB (仅 block 结构体) |
| 首屏精度塑形 | 不变 (已按需) | 不变 |
| Gap 跨域退化 | N/A | 影响 ≤2 行 (precision 时 owned) |

## 7. 风险

- **估算偏差**：字节估算对纯 CJK 文本会高估 3x 行数 → `content_height` 初始偏大 → 首屏 precision pass 后通过 `y_delta` 修正，无视觉影响
- **跨 Gap 文本**：视口内 text 量极小 (~10KB)，偶发的 `into_owned()` 对首屏无感知影响
- **不连续段落**：Novel 模式下段落由连续行合并，`source_range` 已正确覆盖。如有元数据行被跳过，`line_src_ranges` 逐行记录，不受影响
