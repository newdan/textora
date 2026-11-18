# 核心概念与术语

理解本 crate 的关键在于区分**源码表示**、**结构树**、**视觉投影**与**布局几何**四层概念，避免把字节偏移、字素索引和屏幕像素混为一谈。

## 1. 文档结构模型

- **BlockNode（块节点树）**：由 `builder.rs` 从 `MarkdownEvent` 构建的层次结构。包含 `block_range`（在源码中含语法标记的完整字节范围）、`source_range`（内容范围）、`text_lines`（去掉语法标记后的纯内容行）以及 `projected_lines`。
- **BlockSource（块数据源 trait）**：`LazyLayout<S: BlockSource>` 的抽象输入。`MarkdownDoc` 为完整解析树；`NovelStructure` 为大文件小说设计的零拷贝扫描结构（`text_lines` 为空，按需从 `DocView` 读取字节切片）。
- **Container 展开与 `flatten_blocks`**：`BlockKind::Container`、`TableRow_`、`TableCell_` 是结构容器，不直接对应单个 `LaidOutBlock`，布局时会把子块拍平展开。`LazyLayout` 用 `laid_to_doc` 维护拍平后的 `LaidOutBlock` 下标到 `doc.blocks` 下标的映射。

## 2. 坐标与投影体系（最易混淆）

| 概念 | 定义 | 所属模块 |
|---|---|---|
| **Source Byte Offset（源码字节偏移）** | UTF-8 源码字符串中的字节下标。所有持久化修改、Undo/Redo 与 `EditContext.cursor_byte` 均以它为准。 | 全局 / `edit.rs` |
| **SourceAnchor（源码锚点）** | `(byte, CursorAffinity, virtual_grapheme_ordinal)`。在折行边界或折叠文本处，同一个源码字节可能对应两个视觉位置（`Upstream` 上一行末尾 vs `Downstream` 下一行行首）。 | `projection.rs` |
| **Visual Grapheme（视觉字素簇）** | UAX#29 字素簇下标（合并音标、ZWJ emoji 等不拆分）。光标移动、点击命中与选区以字素为单位，由 `VisualGraphemeMap` 保证安全边界。 | `grapheme_map.rs` |
| **ViewPos（视图扁平坐标）** | `(flat_line_idx, grapheme_pos)`，定位到 `LazyLayout.flat_lines` 的某一行及该行内的第几个字素。 | `selection.rs` |
| **Document / Screen 像素坐标** | `LaidOutBlock.rect` 与 `FlatLine.rect` 为文档绝对坐标（含 `y_delta` 修正）；屏幕坐标需减去 `scroll_y` 并加上 `offset_x/offset_y`。 | `layout/types.rs` / `selection.rs` |

**投影（Projection）**：源码字符到视觉显示文本并非 1:1（例如 `**bold**` 折叠了星号、链接折叠了 URL、段首缩进或替换文本产生虚拟/折叠 span）。`projection.rs` 通过 `ProjectionSpanKind`（`Direct` / `Verbatim` / `Collapsed` / `Virtual`）与 `SourceProjectionIndex` 建立**唯一权威的双向映射**，供编辑、选区、搜索高亮与 IME 共用。

## 3. WYSIWYG 展开与物化（Materialize）

- **展开（Expansion）**：纯预览模式下行内标记（`**`、`` ` ``、链接 URL）和块标记（`# `、`- `、`> `）被隐藏。当 `EditContext` 存在且光标/选区落入某个 span 或块内时（闭区间判断 `cursor_in_span`），该标记会“展开”为可见的 `InlineStyle::SourceMarker` 或 `ActiveBlockMarker`，让用户直接编辑源码。
- **物化（`materialize_line`）**：`edit.rs` 根据 `EditContext` 计算某一行在展开/折叠后的最终视觉文本、样式 span 以及 `visual_grapheme_to_source_byte` 映射表。

## 4. 四类空白与可编辑空段落

在 WYSIWYG 模式下，必须严格区分四类空白（详见 `docs/specs/2026-09-07-wysiwyg-block-boundary-invariants.md`）：
1. **语法分隔（HiddenBlockSeparator）**：维持两个 Markdown 块独立所需的首个空行。只负责源码折叠，**不占**独立编辑行。
2. **可编辑空段（Editable Empty Paragraph）**：用户按 Enter 建立的空白正文段落。通过 `EditableParagraphMap` 在 `MarkdownDoc::build_for_editing` 阶段注入为真实 `BlockKind::Paragraph` 节点，特征是 `block_range.is_empty()`、有一条空 `text_line` 与零宽源码锚点。它有真实的 `LaidOutBlock` 和 `FlatLine` 几何，直接参与光标、命中与 IME。
3. **元素间距**：标题 margin、分割线间距、段间距，属于纯排版属性，不参与删除计数。
4. **语法内空内容**：空标题（如 `# `）、空列表项、代码块内空行，按所属块的协议处理。

## 5. 其他子系统核心概念

- **ReplacementPreedit（`view_preedit.rs`）**：当 IME 正在替换一段选区或在代码块内输入组合串（preedit）时，通过独立的临时 `PreviewEngine` 预览替换后的布局，同时保持底层文档坐标为权威。
- **CodeViewport（`layout/code_viewport.rs`）**：代码块不折行时使用的有状态水平滚动视口，光标移动时更新水平偏移，渲染与高亮覆盖层共用同一裁剪区域。
- **MMF（Markdown Mindmap Format）**：基于 Markdown 标题层级（`#` 根、`##` 一级子……）与嵌入式 TOML 属性块（节点属性 / 全局主题）的文本格式，`mmf::Tree` 保存每个节点及子树在源码中的精确字节范围，所有导图交互均规划为对 MMF 源码的原子替换（`EditPlan`）。
