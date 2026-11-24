# 架构概览

## 1. Crate 定位与上下游边界

### 下游调用方（谁使用 `textora-markdown`）
- `crates/app`（可选依赖，通过 `markdown` / `markdown-rich` feature 控制）：注册 `MarkdownEditorViewFactory`、`MarkdownViewFactory`、`NovelViewFactory`、`MindmapPluginFactory`；在事件与剪贴板层调用 `commands::plan_semantic_edit`、`commands::table_structure_capabilities`、`paste::prepare_paste`、`typography::extract_prose_ranges`。
- `crates/notora-app`：注册 `NotoraMarkdownEditorViewFactory`、`MindmapPluginFactory`；调用 `mmf::parser::parse` 与 `typography::extract_prose_ranges`。

### 上游依赖（`textora-markdown` 依赖谁）
- 工作区内部：`core`（`DocView` / `DocViewMut` / 语法高亮 / Unicode UCD）、`ui`（`DrawList`、`Theme`、`Settings`、`ViewPlugin` 等纯 UI 抽象）、`render`（`GlyphVertex`）、`shaping`（HarfBuzz 文本塑形与度量）、`stdext`。
- 外部解析与排版使用 `pulldown-cmark`（CommonMark + GFM 解析）、`unicode-width`、`regex`、`toml`、`encoding_rs`、`url`。字素分割统一使用 `textora_core::unicode`。
- **Feature 隔离**：默认开启 `rich-markdown`（包含 `ratex-*` 数学公式、`mermaid-rs-renderer`、`resvg`、内置轻量 HTML 片段解析器）。关闭时，`embedded` 切换为 `embedded_lite.rs`，HTML 粘贴与公式/图表渲染降级为纯文本或不可用。

### 绝对边界（由 `scripts/check_architecture.sh` 强制）
- `textora-ui`、`textora-appkit-core`、`textora-appkit-shell`、`notora-core` **禁止**依赖 `textora-markdown`。
- 本 crate 通过 `ui::plugin` 定义的 trait（`ViewPlugin`、`PluginFactory`）和纯数据结构（`EditIntent`、`EditAugmentation`、`SemanticEditPlan` 等）与宿主通信，**禁止**引入 `app` 层的 `DocumentView`、`EditorTabSet` 或产品概念。

---

## 2. 核心数据流

### A. 渲染管线（纯预览与小说模式）
```text
源码 (&str)
  → parser::parse_markdown          → ParsedMarkdown (MarkdownEvent 列表)
  → builder::MarkdownDoc::build     → MarkdownDoc (BlockNode 树)
      [NovelView 走 NovelStructure::scan 零拷贝路径]
  → layout::LazyLayout              → 估算高度(全量) → 按视口按需精排(LaidOutBlock) + FlatLine
  → render::render_layout           → ui::core::paint::DrawList
```
- 一次性无状态入口：`lib.rs` 提供 `render_markdown*` 和 `markdown_content_height`。
- 有状态视图入口：`view::BlockViewEngine<S>`（泛型 `S: BlockSource`），被 `MarkdownView`、`NovelView`、`MarkdownEditorView` 复用。

### B. WYSIWYG 编辑闭环（`MarkdownEditorView`）
```text
用户输入 / 按键 / 语义命令
  → augmenter::augment_edit 或 commands::plan_semantic_edit
  → 返回 EditAugmentation / SemanticEditPlan（描述源码替换范围与目标光标）
  → 宿主应用事务并更新 DocView（源码为唯一持久化事实源）
  → 视图收到文档更新 → parse_markdown + MarkdownDoc::build_for_editing
      → 注入可编辑空段落 (EditableParagraphMap)
      → LazyLayout::reuse_unchanged_blocks_from (BlockReconcilePlan 增量复用)
      → ensure_all_blocks (编辑态全量精排) + SourceProjectionIndex 发布
  → 光标/选区/命中/IME 均基于统一投影坐标与 FlatLine 几何计算
```

### C. 智能粘贴子系统（`paste/`）
```text
剪贴板多表示 (PasteRepresentations: markdown / html / rtf / plain / source_url)
  → paste::prepare_paste
      ├─ 若有显式 markdown → 直接采用
      ├─ 若有 html (rich-markdown) → html 解析 → RichDocument
      └─ 否则若有 rtf → rtf 解析 → RichDocument
  → writer::write_markdown → 候选 Markdown
  → selection 模块校验候选可见文字与 plain 是否等价（防丢失/乱序）
      ├─ 等价 → PreparedPaste::Markdown
      └─ 不等或解析失败 → 回退 plain（附 PasteFallbackReason）
```

### D. 思维导图子系统（`mmf/` + `mindmap_view.rs`）
```text
MMF 源码（标题树 + TOML 属性块）
  → mmf::parser::parse → mmf::Tree（含精确字节范围与 MmfDiagnostic）
  → mmf::layout::compute_layout → LayoutTree + HitMap
  → mmf::canvas::render_cards → DrawList（含连线网格缓存）
  → 交互/拖拽 → mmf::edit::plan_* → EditPlan（直接修改 MMF 源码）
```

---

## 3. 模块地图

| 模块 / 文件 | 一句话职责 | 关键类型 / 入口 |
|---|---|---|
| `src/lib.rs` | Crate 入口、无状态渲染 API、feature 切换 | `render_markdown*`, `markdown_content_height` |
| `src/parser.rs` | 封装 `pulldown-cmark`，产出带源码范围的事件流 | `ParsedMarkdown`, `MarkdownEvent`, `MarkdownTag` |
| `src/builder.rs` | 将事件流构建为块树，或扫描小说结构 | `BlockNode`, `BlockKind`, `MarkdownDoc`, `NovelStructure` |
| `src/layout/` | 两阶段懒布局、换行/塑形、块间距、源码行映射、ASCII 图、代码视口 | `LazyLayout`, `LaidOutBlock`, `FlatLine`, `BlockSource`, `SourceLineMap` |
| `src/render.rs`, `render_line.rs`, `style.rs` | 将布局结果转为 `DrawList`；行内样式与主题样式映射 | `render_doc*`, `render_layout*`, `MarkdownStyle` |
| `src/typography.rs` | 识别适合自然中西文混排间距的源码范围 | `ProseRanges`, `extract_prose_ranges` |
| `src/embedded*.rs` | 公式（RaTeX）与 Mermaid 的 SVG/RGBA 光栅化与降级桩 | `EmbeddedRequest`, `EmbeddedImage`, `render_embedded` |
| `src/projection.rs` | 源码锚点与视觉位置之间的双向权威投影索引 | `SourceProjectionIndex`, `SourceAnchor`, `ProjectedText` |
| `src/grapheme_map.rs` | 字符映射转为 UAX#29 字素簇（grapheme）映射 | `VisualGraphemeMap` |
| `src/selection.rs`, `search.rs` | 扁平行选区、点击命中、词边界、搜索高亮矩形缓存 | `ViewPos`, `SelectionState`, `SearchHighlightCache` |
| `src/view/` 及 `src/view_*.rs` | 共享视图引擎与各插件实现；preedit/导航/搜索/拆分历史/代码裁剪 | `BlockViewEngine`, `MarkdownView`, `NovelView`, `MarkdownEditorView` |
| `src/edit.rs` | WYSIWYG 展开上下文与行内标记物化 | `EditContext`, `MaterializedLine`, `materialize_line` |
| `src/augmenter/` | Enter / LineBreak / Backspace / Tab / InsertText 的语法感知增强 | `augment_edit`, `EnterContext`, `classify_enter_context` |
| `src/editable_paragraph*.rs`, `paragraph_leading_spaces.rs` | 编辑器专用空段落的归属映射、增删导航与段首字面空格保留 | `EditableParagraphMap`, `EditableParagraphRun` |
| `src/commands.rs`, `src/commands/` | 语义编辑命令规划（标题、加粗等行内格式、表格结构增删与能力查询） | `plan_semantic_edit`, `table_structure_capabilities` |
| `src/paste/` | 多表示剪贴板转换、等价性校验与 Markdown 输出 | `prepare_paste`, `PasteRepresentations`, `PreparedPaste` |
| `src/mmf/`, `src/mindmap_view*.rs` | MMF 思维导图解析、树布局、卡片渲染、拖拽与源码编辑规划 | `Tree`, `LayoutTree`, `HitMap`, `MindmapView` |
