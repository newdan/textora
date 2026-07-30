# Markdown Preview 美学优化设计方案 (Markdown Rendering Aesthetics Plan)

## 背景描述

当前 `codex/markdown-preview-rendering` 分支已实现基础的排版与渲染解耦（`style.rs`, `layout.rs`, `render.rs`）。然而在视觉美感方面，目前的渲染效果较为骨感，与"优雅"（如 Typora）的要求尚有差距。

本设计方案旨在系统性地提升 Markdown 预览的排版层次感、空间呼吸感和元素精致度，以达到降低认知负荷、提升阅读体验的目标。

## 实施阶段切分 (Phased Implementation Plan)

为了避免互相影响，优化工作将分为四个阶段进行原子化实现。

---

### Phase 1: 空间呼吸感优化 (Spacing & Breathing Room)

目前的标题（Heading）只有下间距，导致标题像平铺在文本中，缺乏层次感。此外，相邻标题之间、段落之间的节奏感也需要精细调整。

#### 1.1 标题上间距

*   **[MODIFY] `crates/markdown/src/style.rs`**
    *   新增 `heading_spacing_top: f32` 属性。
    *   在 `MarkdownStyle::from_theme` 中，将其初始化为 `line_height * 1.5`，确保上方留白大于下方留白（`heading_spacing_bottom = line_height * 0.4`）。
*   **[MODIFY] `crates/markdown/src/layout.rs` — `layout_text_block`**
    *   在 `BlockKind::Heading` 分支中，先让 `ctx.y += ctx.style.heading_spacing_top;`，再进入 `layout_text_block`，最后加上 `heading_spacing_bottom`。

#### 1.2 相邻标题间距折叠 (Adjacent Heading Spacing Collapsing)

当 H2 紧接 H3（或任意两个 Heading 相邻）时，不应简单叠加 `heading_spacing_bottom + heading_spacing_top`，否则会产生过大空白带。规则：两个相邻 Heading 之间的间距取 `max(前者的bottom, 后者的top)`。

同理，Heading → BlockQuote、List → List 等相邻组合也存在空间堆叠问题，但 Heading 的 Margin Collapsing 已能解决 80% 的突兀感。方案将 `last_block_was_heading` 作为轻量入口，后续可按需扩展为更通用的 `last_block_kind` 匹配表。

*   **[MODIFY] `crates/markdown/src/layout.rs`**
    *   在 `LayoutCtx` 中新增 `last_block_was_heading: bool` 标志。
    *   在处理 `BlockKind::Heading` 时，若 `last_block_was_heading` 为 true，则跳过 `heading_spacing_top` 的添加，仅保留 `heading_spacing_bottom`。
    *   每个 Block layout 完成后更新该标志。

#### 1.3 首块标题处理

文档第一个元素如果是 Heading，顶部应适当留白但不宜过大。首块的 `heading_spacing_top` 减半处理。

*   **[MODIFY] `crates/markdown/src/layout.rs`**
    *   在 `LayoutCtx` 中新增 `block_count: usize`，首个 Heading 的 `heading_spacing_top` 乘以 0.5。

---

### Phase 2: 核心视觉元素修复 (Core Visual Fixes)

当前 `BlockQuote` 背景色过深且不透明，`InlineCode` 缺乏背景框，`Table` 缺乏层次。

#### 2.1 引用块 (BlockQuote)

当前 `blockquote_color` 直接用作背景填充色。方案将其重构为极淡的底衬，并通过 RGB 混合（而非 Alpha 衰减，参见"颜色弱化机制"章节）来弱化内部文字颜色，确保次像素抗锯齿不失效。

*   **[MODIFY] `crates/markdown/src/style.rs`**
    *   新增 `blockquote_bg: [f32; 4]`：从原 `blockquote_color` 派生，Alpha 降至 `0.05`（浅色模式）或 `0.08`（深色模式）。背景填充可以用低 Alpha，因为不涉及文字渲染的次像素问题。
    *   保留 `blockquote_color` 作为兼容占位并在注释中标记为 deprecated。
    *   无需单独的 `blockquote_text_color`——文字弱化由 `LayoutCtx.color_fade` 统一驱动（见下文）。
*   **[MODIFY] `crates/markdown/src/layout.rs` — `layout_block` BlockQuote 分支**
    *   对子块递归 layout 前，设置 `ctx.color_fade = 0.25`（文字 RGB 向背景色混合 25%），退出后恢复 `0.0`。`layout_text_block` 通过 `blend_toward_bg()` 计算最终颜色。
*   **[MODIFY] `crates/markdown/src/render.rs`**
    *   将 `dl.fill_rounded` 的填充色从 `blockquote_color` 改为 `blockquote_bg`。

#### 2.2 行内代码 (InlineCode) —— 宽度计算前置到 Layout 阶段

当前 `InlineCode` Span 仅改变文字颜色（`code_color`），无背景框。初版方案曾在 render.rs 中用 `estimate_text_width` 估算宽度来绘制背景框，但这是 Hack：如果宽度低估，文字会溢出背景框；如果高估，背景框会显得突兀。CJK 字符的估算误差尤其严重。

**正确做法**：将宽度计算前置到 Layout 阶段。既然 layout.rs 中已经持有 Shaper，应该让 Layout 产出精准的片段几何信息，render.rs 只做"画笔"。

*   **[MODIFY] `crates/markdown/src/layout.rs` — `LaidOutLine` 结构**
    *   新增 `spans: Vec<LaidOutSpan>` 字段，存储每个行内 StyleSpan 的精确位置和宽度：

    ```rust
    pub struct LaidOutSpan {
        pub text: String,
        pub x_offset: f32,   // 相对于 line rect 的 x 偏移
        pub width: f32,      // Shaper 精确宽度
        pub style: InlineStyle,
    }
    ```

    *   在 `layout_text_block` 中构建 `LaidOutLine` 时，对每个 StyleSpan 调用 Shaper 测量其精确宽度，填充 `LaidOutSpan`。
    *   同时保留 `LaidOutLine.styles: Vec<StyleSpan>` 作为过渡（后续可 deprecated），确保向后兼容。
*   **[MODIFY] `crates/markdown/src/render.rs` — `render_line_with_offset`**
    *   当 `line.spans` 非空时，走"精确路径"：遍历 `LaidOutSpan`，对 `InlineCode` span 用 `dl.fill_rounded` 绘制背景框（`rect.x = line.rect.x + span.x_offset - pad`, `rect.w = span.width + pad * 2`），再绘制文字。
    *   当 `line.spans` 为空时，回退到现有 `styles` 路径（兼容旧数据）。
    *   `estimate_text_width` 函数标记为 deprecated，最终目标是从 render.rs 中彻底移除。
*   **[MODIFY] `crates/markdown/src/style.rs`**
    *   新增 `inline_code_padding: f32`，默认 `font_size * 0.15`，统一控制 InlineCode 背景框的左右内边距。

#### 2.3 表格斑马纹与增强 (Table Striping)

*   **[MODIFY] `crates/markdown/src/style.rs`**
    *   新增 `table_stripe_bg: [f32; 4]`：浅色模式 `#F8F8F8`（极浅灰），深色模式轻微提亮的深色。
    *   新增 `table_header_separator_thickness: f32`：Header 底部分隔线宽度，默认 `2.0`。
*   **[MODIFY] `crates/markdown/src/layout.rs` — `LaidOutBlockKind::Table`**
    *   在 Table variant 中新增 `row_heights: Vec<f32>` 字段，存储每一行（header + body）的精确高度，避免渲染阶段重复计算。当前只有 cell line 的 rect，反推行高需要遍历，提前记录更干净。
*   **[MODIFY] `crates/markdown/src/render.rs` — Table 渲染**
    *   Header 行：使用 `table_header_bg` 填充，底部用 `table_header_separator_thickness` 绘制加粗分隔线。
    *   Body 行：奇数行（`row_index % 2 == 1`）使用 `table_stripe_bg` 绘制整行背景。
    *   Body 行间：用 `1.0px` 细线分隔。

---

### Phase 3: 字体排印基础完善 (Typography Foundations)

当前底层绘制 `DrawList::text` 以及 `Shaper` 未传入字重（Weight）和样式（Style），导致粗体和斜体无法显示。未区分正文无衬线字体与代码等宽字体。

本阶段涉及两个独立路径，建议分步实施：

#### 3.1 字体族区分（低风险，立即可做）

*   **[MODIFY] `crates/markdown/src/style.rs`**
    *   引入 `body_font_family: Option<String>` 和 `code_font_family: Option<String>`。
    *   在 `from_theme` 中初始化：`body_font_family` 从系统 UI 字体获取，`code_font_family` 设为等宽字体（如 `"JetBrains Mono"` → `"Menlo"` → `"monospace"` 的 fallback 链）。
*   **[MODIFY] `crates/markdown/src/render.rs` — `render_line_with_offset`**
    *   根据 `line.is_code` 选择对应的 `font_family` 传入 `dl.text()`。当前 `DrawCmd::Text` 已支持 `font_family` 字段，无需底层变更。

#### 3.2 Bold/Italic 渲染路径（路径 A：纯渲染，推荐先做）

不改变 Shaper，仅在渲染时传递 weight/style 信息给底层图形栈，由字体系统选择正确的字体文件绘制。

*   **[MODIFY] `crates/ui/src/core/paint.rs`**
    *   扩展 `DrawCmd::Text` 结构，增加 `weight: Option<FontWeight>` 和 `style: Option<FontStyle>` 字段。
    *   新增枚举 `FontWeight`（`Thin`, `Normal`, `Bold`, 等）和 `FontStyle`（`Normal`, `Italic`）。
*   **[MODIFY] `crates/markdown/src/render.rs` — `render_line_with_offset`**
    *   解析 `StyleSpan` 中的 `InlineStyle::Bold` 和 `InlineStyle::Italic`，将对应的 weight/style 参数传入 `dl.text()`。
*   **降级策略（合成粗体）**：当底层字体文件缺少 Bold 字重时，在同一位置 x 方向微偏移（+0.5px）再绘制一次同色文字，模拟粗体效果。此逻辑应实现在底层 `DrawList::text` 或对应的渲染后端中。

#### 3.3 Shaper 感知路径（路径 B：后续优化）

Shaper 也需要知道 weight/style，因为字重/倾斜会影响字形宽度，进而影响 word wrap 精度。

*   **[MODIFY] `crates/shaping/src/lib.rs`**
    *   修改 `Shaper` API，新增 `set_font_weight(weight: FontWeight)` 和 `set_font_style(style: FontStyle)`。
    *   在 `shape()` 方法内部，构建 `Attrs` 时调用 `.weight()` 和 `.style()`。
*   **[MODIFY] `crates/markdown/src/layout.rs` — `wrap_text`**
    *   在调用 shaper 前，根据当前 span 的 style 设置对应的 weight/style。
*   **标注**：路径 B 仅在路径 A 完成后、word wrap 精度成为可见问题时再实施。

#### 3.4 已知局限与淘汰路径

`render.rs` 中的 `estimate_text_width` 目前用于 styled text 渲染时光标步进计算（`cursor_x += w`）。Phase 2.2 引入的 `LaidOutSpan` 机制已经将宽度测量前置到 Layout 阶段，至此 render.rs 不再需要 `estimate_text_width`。Phase 3 完成 `LaidOutSpan` 全线切换后，该函数可彻底移除。

在 `LaidOutSpan` 未完全覆盖所有渲染路径之前（如回退路径），`estimate_text_width` 对 weight/style 不感知的偏差仍然存在，但不影响视觉正确性（仅影响后续 span 的 x 偏移量），属于非阻塞项。

---

### Phase 4: 高级代码块增强 (Code Block Enhancements) [远期规划]

当前 `CodeBlock` 全是统一颜色。

#### 4.1 语法高亮后端选型

*   **方案 A：syntect**（推荐）
    *   基于 Sublime Text 语法定义（`.sublime-syntax`），社区积累丰富。
    *   集成简单：纯 Rust 解析，无需编译 grammar。
    *   适合编辑器预览场景（非超长文档）。
*   **方案 B：tree-sitter**
    *   更快，但需要为每种语言编译 grammar。
    *   适合 AST 级代码分析场景，对预览来说过重。

推荐 syntect 作为首选方案，预留 tree-sitter 替换接口。

#### 4.2 实现路径

*   **[NEW DEPENDENCY] `Cargo.toml`**：添加 `syntect`。
*   **[MODIFY] `crates/markdown/src/builder.rs` — `MarkdownDoc::build`**
    *   解析 `CodeBlock` 时，对代码文本运行 syntect 高亮。
    *   生成带有语义颜色的 `StyleSpan`（`InlineStyle::HighlightedToken { color: [f32; 4] }`），替换当前的纯文本 line。
*   **[MODIFY] `crates/markdown/src/style.rs`**
    *   新增语法高亮颜色槽位：`syntax_keyword`, `syntax_string`, `syntax_comment`, `syntax_type`, `syntax_number`, `syntax_function`, `syntax_variable`。
    *   从编辑器 Theme 中派生这些颜色（或从 syntect 主题映射）。
*   **[MODIFY] `crates/markdown/src/render.rs`**
    *   `render_line_with_offset` 中识别 `HighlightedToken` span 并使用对应颜色渲染。

---

### Phase 5: 补充视觉细节 (Polish & Edge Cases) [新增]

#### 5.1 图片占位符美化

commits 中已引入 "image placeholder"，但缺少美学处理：
*   占位符使用圆角矩形 + 虚线边框，区别于内容区域。
*   Alt text 居中显示在占位符内，使用略淡的文字颜色。
*   可选的图片尺寸标注（如 "800×600"）在占位符右下角。

#### 5.2 链接交互视觉

*   当前：仅静态下划线。
*   建议：链接在 hover 时从 `link_color` 变为更亮的强调色，或添加底部轻微高亮背景（underline → highlight 过渡）。
*   需要 `LaidOutLine` 中额外记录链接的 `url` 信息用于 hit testing（如果尚未记录）。

#### 5.3 选择高亮样式

用户在预览区选中文字时的 `::selection` 颜色应与编辑器正文区保持一致，从 Theme 中读取 `selection_bg` 和 `selection_fg`。

#### 5.4 列表视觉微调

*   嵌套列表的 `level_indent` 应根据层级递增（当前只有一级 indent）。
*   TaskList 的 checkbox 未选中状态：用更淡的边框色（`text_color` alpha 降至 0.4），与已选中形成对比。
*   有序列表的数字使用等宽数字（tabular figures）：通过 OpenType `FontFeature::TabularNumbers`（`tnum`）使所有数字字符占据相同宽度。这是排版中极易被忽略但对齐感影响巨大的细节——许多现代编辑器（包括早期 VSCode）都会在此翻车，导致 `9.` 和 `10.` 的小数点无法垂直对齐。

---

## 架构补充：颜色弱化机制（RGB 混合，非 Alpha 衰减）

BlockQuote 和未来可能出现的 Callout/Admonition 都需要对子树文字做颜色弱化。初版方案曾在 `LayoutCtx` 中用 Alpha 系数衰减文字透明度，但存在图形学问题：**文本带 Alpha < 1.0 会导致次像素抗锯齿（Subpixel Anti-aliasing，如 ClearType / FreeType RGB subpixel rendering）失效**，字体边缘会变得模糊或发虚。

**正确做法**：保持 Alpha 恒为 1.0，通过 RGB 通道向背景色混合来实现弱化。

```rust
/// 将颜色按比例向目标色混合，保持完全不透明。
/// ratio: 0.0 = 保持原色, 1.0 = 完全变成目标色
fn blend_toward(color: [f32; 3], target: [f32; 3], ratio: f32) -> [f32; 3] {
    [
        color[0] + (target[0] - color[0]) * ratio,
        color[1] + (target[1] - color[1]) * ratio,
        color[2] + (target[2] - color[2]) * ratio,
    ]
}
```

示例：正文色 `#333333`，背景色 `#FFFFFF`，弱化 25% → `blend_toward([0.2,0.2,0.2], [1.0,1.0,1.0], 0.25)` → `#404040`。Alpha 保持 1.0，文字渲染锐利。

*   **[MODIFY] `crates/markdown/src/style.rs`**
    *   新增 `blend_toward_bg(color: [f32; 4], bg: [f32; 4], ratio: f32) -> [f32; 4]` 工具函数（仅混合 RGB，Alpha 始终为 1.0）。
*   **[MODIFY] `crates/markdown/src/layout.rs` — `LayoutCtx`**
    *   新增字段：
    ```rust
    color_fade: f32,             // 0.0 = 正常, 1.0 = 完全背景色
    background_color: [f32; 4],  // Theme 背景色，用于 blend 目标
    ```
    *   `layout_text_block` 在设置 `LaidOutLine.color_override` 时调用 `blend_toward_bg(color, bg, color_fade)`。
    *   BlockQuote 分支在递归子块 layout 前设置 `color_fade = 0.25`（向背景色混合 25%），退出后恢复 `0.0`。

---

## 性能考量

Phase 2 的 InlineCode 背景框和表格斑马纹会增加 DrawList 中的 draw calls。粗略估算：
- 短文档（< 200 行）：影响可忽略
- 长文档（5000+ 行，含大量 InlineCode）：cmds 数量可能从 ~500 增长到 ~2000

当前 `render_doc` 已使用 `PushClip` 做 viewport clipping，但 cmds 仍然全部 push 进去。如果性能成为问题，后续可引入 render 阶段的 Y 轴粗筛（在 `render_block_with_offset` 入口处判断 block rect 是否与 viewport 有交集）。此优化不在本方案范围内，仅作为备忘。

---

## 待确认事项与风险点

*   **关于字体库支持 (Phase 3)**：当前系统的 `Shaper` 和 `ui::core` 是否已具备加载同一字体族下不同字重（如 Roboto-Bold, Roboto-Italic）的能力？如果尚未具备，Phase 3 路径 B 可能涉及更底层的字体加载器（FontLoader/cosmic-text）的改造。路径 A（纯渲染路径）不依赖此能力，建议先行。
*   **关于颜色系统**：当前 `MarkdownStyle` 中颜色字段为裸 `[f32; 4]`，随着字段增多（`blockquote_bg`, `table_stripe_bg`, 语法 token 颜色），可考虑将其抽取为 `MarkdownColors` 子结构体，保持 `Style` 的清晰度。注意文字弱化颜色（如 BlockQuote 内文字）不再作为独立颜色字段存储，而是由 `blend_toward_bg` 在 Layout 阶段动态计算。
*   **关于阶段执行**：Phase 1 和 Phase 2 是纯视觉和布局逻辑的调整，不涉及底层基础设施，可以立即实现并快速看到显著的美学提升。Phase 5 可作为散布在 Phase 1-4 中顺手处理的细节项。
