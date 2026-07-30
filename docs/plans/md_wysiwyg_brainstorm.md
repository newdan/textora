# Markdown 所见即所得 (WYSIWYG) 编辑能力方案脑暴

基于目前的 `core` (底层文档与光标)、`markdown` (纯预览与渲染布局) 及 `ui` 架构，要实现类似 Typora 的 "实时预览+原地编辑" 体验，我们需要完成从 **"单向渲染"** 到 **"双向映射与基于光标状态的动态布局"** 的演进。

## 核心痛点与现状分析
1. **AST 信息丢失**：目前 `markdown::parser` 使用 `pulldown_cmark` 解析，但在 `builder.rs` 构建 `BlockNode` 时，丢失了精确的源码字节范围 (Source Spans)，并且诸如 `*`, `#`, `[`, `]` 等 Markdown 语法字符已被直接丢弃，布局层只剩下纯文本和 `StyleSpan`。
2. **渲染层无状态**：`PreviewEngine` 只是一个被动的展示面板，它并不感知 `Document` 的光标位置 (Cursor Position) 和选区 (Selection)，仅负责将缓冲区文本映射为 `LaidOutDoc` 视觉块。
3. **输入流转**：目前所有的编辑交互绑定在基于行的文本编辑器 UI 上，而 Markdown Preview 无法直接截获和应用键盘事件至底层 `TextBuffer`。

---

## Typora 式体验的核心机制
- **全局富文本 (Rich Text)**：当光标不在该区域时，隐藏一切不必要的 Markdown 标记，直接展示排版后的富文本视图。
- **局部源码展开 (Focus Unfold)**：当光标移入某个特殊区域（如加粗、斜体、链接、标题）时，自动恢复显示该区域的 Markdown 源码字符，允许用户像在普通文本编辑器里一样修改源码。
- **无缝切换**：光标移出时，源码再次折叠为富文本，变化过程自然且实时。

---

## 架构演进路线 (Phases)

### Phase 1: AST 与源码的精确双向映射 (Source Spans)
要实现任何编辑交互，视图中的每一个字块都必须知道自己对应在 `TextBuffer` 中的 `Range<usize>`。
1. **修改 `MarkdownEvent` 与 `BlockNode`**：在 `parser.rs` 的 offset 迭代器中，捕获每个元素的起止位置，并传递到 `builder.rs`，给 `BlockNode` 增加 `pub span: Range<usize>`。
2. **内联样式的映射**：`StyleSpan` 必须保留在源码中的真实偏移，而不能仅是纯文本结果的偏移。
3. **坐标双向查询**：
   - `screen_pos_to_byte_idx(x, y)`：用于鼠标点击定位光标。
   - `byte_idx_to_screen_pos(idx)`：用于渲染光标、输入法候选框 (IME) 定位。

### Phase 2: 基于光标的动态布局 (Dynamic Layout based on Cursor)
需要将 `cursor_byte_idx` 传递给 `markdown::layout::LazyLayout`，使其成为布局计算的参数之一。
有两条技术路线：
- **方案 A (Block-Level Unfold，类似黑曜石早期)**：
  布局时判断：如果光标落入某个 `BlockNode`（如 Heading 或 CodeBlock），则该 Block 退化为纯文本源码渲染（直接从底层 Buffer 截取对应 Span 的字符串）；否则，进行富文本布局。
  *优点*：易于实现，不破坏现有 `LaidOutLine` 逻辑。
  *缺点*：光标进入大段落时，整个段落闪烁回源码形态，体验较差。
- **方案 B (Inline-Level Unfold，Typora 原生体验)**：
  `BlockNode` 始终按富文本布局。但是在布局行内元素（如 `**bold**`）时，判断光标是否在该 `StyleSpan` 的边缘或内部。如果在，则向塑形文本 (Shaping Text) 中临时 **注入** `**` 字符；否则隐藏它们。
  *难点*：这要求 `layout.rs` 极其动态，文本宽度会随着光标移动而突变，可能导致频繁的换行重新计算 (Re-wrap)。

### Phase 3: 打通输入与更新流水线 (Edit Pipeline)
1. **输入劫持**：Markdown 视图组件需要能获得焦点 (Focus)，并响应 `KeyDown`, `TextInput` 等事件。
2. **事件转换**：将键盘输入转化为底层的 `TextBuffer` 修改命令（调用 `Document::insert` 或 `Document::delete`）。因为有前面建立的精确 Span 映射，我们能保证修改正确的位置。
3. **增量更新 (Incremental Updates)**：
   每次键盘输入，如果全量调用 `parse_markdown` 和全量重新 `shaping` 会带来灾难性的性能问题。
   由于目前已经有了 `LazyLayout`（带有两阶段计算），我们需要在此基础上加入 **增量 AST 解析**：
   - 识别出被修改的 `BlockNode`。
   - 仅截取该段落对应的源码重新送入 `pulldown_cmark` 进行局部解析。
   - 替换 AST 节点，并使该 Block 及其后续 Block 的 `y_delta` 失效，触发局部重绘。

### Phase 4: 难点控件特殊处理 (Tables & Images)
- **表格 (Table)**：纯文本 Markdown 表格的源码往往因为对齐而充满空格和竖线。如果退回源码编辑极难阅读。可以考虑当光标进入 `TableNode` 时，不展开源码，而是在当前单元格上直接悬浮一个微型 `TextBox` 供用户输入纯文本，然后自动反向生成带对齐的 Markdown 表格源码覆盖回去。
- **图片与媒体**：类似地，点击图片不展示 `![alt](url)`，而是弹出一个编辑浮窗修改 URL 和属性。

---

## 最小可行性产品 (MVP) 实施建议
为避免改动过大导致架构崩塌，建议采用以下敏捷步骤：
1. 先在 `builder.rs` / `layout.rs` 里打通 `SourceSpan`（能根据点击位置打印出底层 Markdown 原文的位置）。
2. 让 `Preview` 视图能展示闪烁的光标（仅展示，不破坏任何现有布局），打通 `byte_idx_to_screen_pos`。
3. 实现简单的 Block-level Unfold：点击标题，标题变为 `# 标题` 的纯文本；点击外部，恢复大字号。
4. 接通键盘输入，允许在 Block 展开的状态下打字。
5. 最后再慢慢向 Inline-level Unfold 迭代。

---

## 架构决策：复用现有 mdview 还是新增编辑视图？

考虑到 `novel` 已经是只读的且与 `markdown` 共用了一套底层渲染逻辑（`PreviewEngine`），在引入编辑能力时，我们面临两种选择：**A. 新增独立的 MarkdownEditorView**，或者 **B. 直接在原有的 mdview (PreviewEngine) 基础上扩展编辑能力**。

**强烈建议选择方案 B（在原有的 mdview 基础上增加编辑能力，通过 Flag 控制模式）**。

### 为什么选择方案 B (复用 mdview 增强)？

1. **渲染代码极度重合**：
   无论是纯只读的 Novel 渲染，还是 Typora 式的所见即所得编辑，它们**非光标所在区域的富文本排版算法是完全一致的**（如文本折行、多级标题缩进、表格绘制等）。如果拆分成两个视图，我们将不得不维护两套几乎相同的 `Layout` 引擎和 `Shaper` 逻辑，这严重违背了 DRY (Don't Repeat Yourself) 原则。
2. **渐进式降维打击**：
   所见即所得编辑器的布局引擎可以看作是“带有光标感知能力的超级渲染引擎”。只要我们给原有的 `PreviewEngine` 传入 `cursor_pos = None` 或者设置 `is_read_only = true`，它就自动退化回了现在极其轻量、高效的纯预览引擎，天然满足 `novel` 分栏和普通 md 预览的需求。
3. **架构收敛**：
   将所有的 Markdown/Novel 渲染统一在 `PreviewEngine` 下，任何底层排版细节的优化（比如修正表格对齐、优化图片渲染）都会自动让“只读视图”和“编辑视图”同时受益。

### 如何在方案 B 下保持代码整洁 (分离读写关注点)？

为了避免把纯只读的逻辑和复杂的输入处理混在一起，我们可以在架构上做如下隔离：

1. **UI 层隔离 (View/App 层)**：
   保留 `MarkdownPreviewView` (供纯预览使用，忽略所有键盘输入事件)，新增一个 `MarkdownEditorView`。这两个 View 内部**组合 (Compose)** 相同的 `PreviewEngine` 实例。
   - `MarkdownPreviewView` 负责将其只读参数传给 `PreviewEngine`。
   - `MarkdownEditorView` 负责处理键盘事件、拦截输入、调用 `Document::insert/delete`，并将最新的 `cursor_byte_idx` 传递给 `PreviewEngine`。
2. **核心引擎层的状态下放**：
   `PreviewEngine` 和 `LazyLayout` 增加一个可选的上下文 `EditContext { cursor_span: Range<usize> }`。
   - `novel` 调用时，不传入 `EditContext`，布局引擎走最快的静态全量布局分支。
   - 编辑器调用时，传入 `EditContext`，布局引擎触发局部展开 (Unfold) 逻辑。
