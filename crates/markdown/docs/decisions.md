# 关键设计决策记录

本页浓缩影响本 crate 结构的核心设计取舍，并索引上游详细 spec（位于工作区 `docs/specs/` 与 `docs/`）。

## ADR-01：源码为唯一持久化事实源，不维护独立可变富文本树
- **决策**：WYSIWYG 编辑与 MMF 思维导图都不把 AST/Block 树作为可编辑主模型；所有按键增强、命令、导图拖拽均规划为源码字节替换事务，提交后重新解析与增量重排。
- **理由**：保证 Markdown/MMF 源码保真度、Undo/Redo 原子性，以及多视图（源码视图与 WYSIWYG 视图）切换零同步成本。
- **依据**：`docs/specs/2026-09-07-wysiwyg-block-boundary-invariants.md`、`docs/specs/2026-07-12-mmap-wysiwyg-editing-design.md`。

## ADR-02：可编辑空段落统一进入布局流，而非局部几何补偿
- **决策**：编辑态通过 `MarkdownDoc::build_for_editing` 注入零宽源码范围的 `BlockKind::Paragraph` 节点，使其走正常 `LaidOutBlock` / `FlatLine` / `SourceProjectionIndex` 管线；隐藏语法分隔仅由 `SourceLineMap` 负责源码折叠。
- **理由**：此前靠 `y_delta` 补偿和 `SourceLineMap` 反推空行位置导致光标、IME、点击命中与删空往返不一致。统一进入布局后所有几何共享同一事实源。
- **依据**：`docs/specs/2026-09-06-editable-paragraph-layout.md`、`docs/specs/2026-09-07-wysiwyg-block-boundary-invariants.md`。

## ADR-03：两阶段懒布局 + `BlockSource` 抽象复用同一引擎
- **决策**：`LazyLayout<S: BlockSource>` 先做全量高度估算，再按视口按需精排（编辑态调用 `ensure_all_blocks`），并通过 `BlockReconcilePlan` 增量复用未变更块。`MarkdownDoc` 与零拷贝的 `NovelStructure` 共用同一引擎。
- **理由**：兼顾大文档/小说打开首帧速度与编辑态精确几何。

## ADR-04：单一权威源码投影与 UAX#29 字素坐标
- **决策**：引入 `SourceProjectionIndex`（支持 `CursorAffinity` 区分折行/折叠边界）与 `VisualGraphemeMap`，统一服务于光标、选区、点击命中、搜索高亮与 IME 替换预编辑（`ReplacementPreedit`）。
- **理由**：避免搜索、选区和光标各自计算投影偏移导致的错位与多码点字素拆分 Bug。

## ADR-05：纯 Rust 嵌入渲染 + Feature 隔离
- **决策**：公式（`ratex-*`）与图表（`mermaid-rs-renderer` + `resvg`）封装在 `embedded.rs` 中，通过 `rich-markdown` feature 隔离，关闭时切换为 `embedded_lite.rs`；渲染失败或超限一律退回源码。
- **理由**：无需外部浏览器、Node 或 TeX 运行时，同时允许轻量构建裁剪重依赖。
- **依据**：`docs/specs/2026-09-26-markdown-math-mermaid.md`。

## ADR-06：智能粘贴“内容优先 + 确定性等价校验”
- **决策**：同步采集剪贴板多表示，按 Markdown → HTML → RTF 优先级转换；只要存在纯文本 `plain`，富文本转换结果必须通过可见文字等价校验，否则回退纯文本。不保存本地/内嵌图片。
- **理由**：防止浏览器复杂 HTML 导致正文丢失或乱序，避免异步竞态与本地附件垃圾。
- **依据**：`docs/specs/2026-09-02-smart-markdown-paste-design.md`。

---

## 上游 Spec 与审查文档索引

| 主题 | 文档路径（相对于工作区根目录） |
|---|---|
| WYSIWYG 空行与块边界不变量 | `docs/specs/2026-09-07-wysiwyg-block-boundary-invariants.md` |
| 可编辑空段落统一布局 | `docs/specs/2026-09-06-editable-paragraph-layout.md` |
| 段落编辑语义与缩进 | `docs/specs/2026-09-06-wysiwyg-paragraph-edit-semantics.md`, `docs/specs/2026-09-07-wysiwyg-paragraph-indentation.md` |
| Enter / Backspace 与快捷键 | `docs/specs/2026-08-02-markdown-wysiwyg-enter-backspace-behavior.md`, `docs/specs/2026-08-19-markdown-editor-keybindings.md` |
| WYSIWYG 表格编辑 | `docs/specs/2026-09-24-markdown-wysiwyg-table-editing-design.md` |
| 空文档光标、DPI 与 IME Preedit | `docs/specs/2026-07-23-markdown-empty-document-caret-design.md`, `docs/specs/2026-07-24-markdown-empty-document-dpi-typography-design.md`, `docs/specs/2026-07-24-markdown-empty-document-ime-preedit-design.md` |
| 块间距与中西文混排间距 | `docs/specs/2026-09-10-markdown-block-spacing-design.md`, `docs/specs/2026-09-20-cjk-ascii-prose-spacing.md`, `docs/specs/2026-09-20-markdown-font-consistency.md` |
| 公式与 Mermaid 渲染 | `docs/specs/2026-09-26-markdown-math-mermaid.md` |
| ASCII 图表渲染 | `docs/specs/2026-07-15-ascii-diagram-border-snapping-design.md`, `docs/specs/2026-07-15-ascii-diagram-grid-rendering-design.md` |
| 智能粘贴设计 | `docs/specs/2026-09-02-smart-markdown-paste-design.md` |
| 思维导图（MMF）规范与交互 | `docs/markdown_mindmap_format_v0.1.md`, `docs/specs/2026-07-12-mmap-wysiwyg-editing-design.md`, `docs/specs/2026-07-15-mindmap-collapse-design.md` |
