# Markdown 预览渲染设计

## 概述

为 .md 文件新增预览模式，解析 markdown 源码并以富文本样式渲染。对标 zed 的 markdown crate，采用 Element Tree 架构。

## 交互方式

- 编辑/预览切换：同一 view 在源码编辑和预览之间切换
- 快捷键：`Cmd+\` 或 `Cmd+Shift+P`
- 预览为只读视图，支持滚动

## 首版范围

- 标题 h1-h6、段落、粗体、斜体、行内代码、代码块
- 无序列表、有序列表、任务列表（checkbox 渲染但不可交互）
- 引用块、水平线、链接（渲染但不打开）、脚注
- 表格、图片（显示 alt text 占位）
- 不包含：Mermaid 图、HTML block、语法高亮

## 架构

```
crates/markdown/          （新 crate）
├── parser.rs             pulldown_cmark 封装，产出事件流
├── builder.rs            MarkdownBuilder，对标 zed MarkdownElementBuilder
├── style.rs              MarkdownStyle 配置
├── render.rs             遍历 builder 产物 → 生成 DrawList
└── lib.rs                公开 API

crates/app/src/
└── md_preview.rs         预览视图：view mode 切换、滚动、鼠标事件
```

### 数据流

```
源码 → parser::parse(src) → ParsedMarkdown { events, ... }
  → MarkdownBuilder → MarkdownDoc { block_tree, lines, links }
  → layout pass（计算每个 block 的位置/尺寸）
  → render pass（遍历 laid-out blocks → DrawList）
  → app::paint_backend → wgpu
```

与 zed 的关键差异：zed 依赖 GPUI 的 layout engine 计算位置，我们需要自己写 layout pass。

## 关键类型

### Parser 事件

对标 zed parser，精简去掉 HTML/Mermaid/MetadataBlock：

- `MarkdownEvent`: Start(tag) | End(tag) | Text | Code | SoftBreak | HardBreak | Rule | TaskListMarker(bool) | FootnoteReference
- `MarkdownTag`: Paragraph | Heading | BlockQuote | CodeBlock | List | Item | Table | TableHead | TableRow | TableCell | Emphasis | Strong | Strikethrough | Link | Image | FootnoteDefinition

### Builder (builder.rs)

对标 zed `MarkdownElementBuilder`：

```
block_stack: Vec<BlockNode>          // 代替 zed 的 div_stack
pending_line: PendingLine            // 积累中的文本
rendered_lines: Vec<RenderedLine>    // 成品文本行
text_style_stack: Vec<TextStyleMod>  // 代替 zed 的 TextStyleRefinement 栈
rendered_links: Vec<RenderedLink>    // 用于 hit testing
table: TableState                    // 直接搬 zed
code_block_stack: Vec<bool>          // 追踪是否在代码块内
list_stack: Vec<ListStackEntry>      // 有序/无序列表状态
```

**BlockNode** 代替 GPUI 的 AnyDiv：
```rust
struct BlockNode {
    kind: BlockKind,           // Container|Heading|Paragraph|CodeBlock|BlockQuote|...
    children: Vec<BlockNode>,
    source_range: Range<usize>,
}
```

**TextStyleMod** 是 inline 样式标记：Bold | Italic | Strikethrough | InlineCode | Link { url } | Heading { level } | BlockQuote | CodeBlock

### 文本行 (RenderedLine)

对标 zed：`text: String` + `runs: Vec<TextRun>` + `source_mappings: Vec<SourceMapping>` + `source_end: usize`

### MarkdownStyle

适配本项目绘制系统：
- 字体：base/mono 字号、字体族、行高
- 颜色：text, heading, code_bg, code_text, blockquote_border, blockquote_bg, link, table_border, table_header_bg, table_stripe_bg, rule
- 标题缩放：h1-h6 的字号比例
- 布局参数：blockquote_indent, list_indent, code_padding

## Builder 方法对标表

| zed | 我们 | 说明 |
|-----|------|------|
| push_text_style | push_text_style | 压入 TextStyleMod |
| pop_text_style | pop_text_style | 弹出 |
| push_div | push_block | BlockNode 代替 AnyDiv |
| pop_div | pop_block | flush_text + 挂到父节点 |
| push_text | push_text | 追加到 pending_line |
| flush_text | flush_text | pending_line → RenderedLine |
| push_link | push_link | 记录 link 信息 |
| push_list / next_bullet_index | 相同 | 列表状态管理 |
| TableState | 直接搬 | 表格行列追踪 |

## 事件循环

和 zed 几乎一样的遍历模式。事件匹配：

- Heading → push TextStyleMod::Heading + push BlockKind::Heading
- Strong/Emphasis → push/pop TextStyleMod::Bold/Italic
- CodeBlock → push TextStyleMod::CodeBlock + push BlockKind::CodeBlock
- Table → TableState.start + push BlockKind::TableWrapper
- List → push_list + push BlockKind::Container
- Item → next_bullet_index + push BlockKind::ListItem
- Link → push_link + push TextStyleMod::Link
- Text/Code → push_text

## Layout Pass

遍历 BlockNode 树 + RenderedLine，计算每个 block 的坐标：

1. 文本类 block：用 text shaper 做 word wrap，按 viewport 宽度分行
2. 代码块：monospace 字体，不 wrap（横向可滚动）
3. 引用块/列表：左侧缩进，递归 layout 子 block
4. 表格：先算各列最大宽度 → 分配列宽 → layout 每格
5. 累积 y 坐标，填入每个 block 的 Rect
6. 输出包含剪裁信息（viewport 外不渲染）

## Render Pass

LaidOutBlock → DrawList：

| 元素 | DrawCmd |
|------|---------|
| 标题 | Text（大字号 bold） |
| 段落 | Text（按 run 样式逐段，bold/italic 提供不同字体变体） |
| 行内代码 | FillRect（背景色）+ Text（mono） |
| 代码块 | FillRect（bg）+ PushClip + Text（mono）+ PopClip |
| 引用块 | FillRect（淡背景）+ StrokeRect（左边竖线） |
| 表格 | 循环 StrokeRect（网格线）+ Text（单元格内容） |
| 水平线 | StrokeRect（全宽横线） |
| 链接 | Text（特殊颜色 + 下划线） |
| 任务列表 | StrokeRect（checkbox 框）+ 可选 FillTriangle（checkmark） |

## App 层集成

- `MarkdownPreview` struct：持有 source、LaidOutDoc 缓存、scroll 状态、style
- `ViewMode` 枚举新增 `MarkdownPreview` 变体
- viewport 宽度变化时重新 layout
- 源码变化（编辑后切回预览）时重新 parse+layout
- 点击事件通过 source_mappings 反查，暂不处理链接点击

## 后续扩展

- Mermaid 图表
- 链接点击（系统浏览器）
- 代码块语法高亮
- 图片加载（本地文件）
- 增量更新（编辑后不重新解析整个文档）
