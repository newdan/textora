# 大文件“彻底秒开”架构改造方案

## 背景与问题

在当前的架构中，打开一个包含 10 万行文本（约 5MB）的超大型长篇小说时，UI 渲染仍存在大约 240 毫秒的冷启动感知耗时。经性能分析，核心瓶颈与问题如下：

1. **AST 的海量无效内存分配**：
   底层解析 5MB 的纯文本或 Markdown 实际上仅需几十毫秒，但程序在生成 AST（`MarkdownDoc`）时，为每行文本均分配并拷贝了完整的 `String`（即 `text_lines: Vec<String>`）。这导致解析过程中出现了海量的堆内存分配开销（占据了约 38 毫秒，并在后续增加 GC 和拷贝负担）。
2. **非可见区域的无效解析**：
   Grapheme 分词和排版前置处理伴随了全量 AST 构建，而实际上首屏仅需展现寥寥数十行文本。
3. **接口语义存在跨层泄漏**：
   目前底层的 `DocView` 暴露了 `Cow<'_, [u8]>`，迫使纯文字渲染的上层 UI 引擎接触字节层概念，产生了解耦边界上的模糊。

---

## 解决方案：零拷贝按需 AST (Zero-Copy On-Demand AST)

彻底摒弃 AST 中保存全量真实文本数据的做法。我们将 AST 节点弱化为轻量级的“字节区间（Range）映射表”，实现真正的“看到哪儿，去底层读哪儿”。

### 核心设计点

1. **零拷贝 `BlockNode`**
   - AST 中的段落节点不再持有 `Vec<String>`，全部替换为 `source_range: Range<usize>`。
   - 解析（无论是 `pulldown-cmark` 还是小说的按标点分段）只记录文本起止位置，构建 10 万行 AST 的速度将被压缩到数毫秒内。

2. **按需 Grapheme 拆解与排版**
   - 渲染管线仅在首屏（及上下缓冲区域）调用 `ensure_precise_range` 时，才拿着可视区域的 `Range` 向底层提取实际文本。
   - 这意味着极度耗时的 HarfBuzz 塑形与 Grapheme 分词过程，被严格限制在了“视口可见区域”内。

3. **纯粹的抽象边界 (`Cow<'_, str>`)**
   - 升级 `core::document::DocView` 的接口语义，将其从返回 `[u8]` 变更为返回 `Cow<'_, str>`。
   - 保证底层数据的字节与 UTF-8 校验完全在 App 层内闭环。在将底层 Buffer 的视图转化为 `str` 时，**严格禁止使用 `from_utf8_unchecked`**（遵循项目代码规范），必须使用 `std::str::from_utf8(bytes).expect("AST range must align with valid UTF-8 character boundaries")` 进行安全断言，向 UI 渲染层输出纯粹的文本引用。

---

## 拟定架构调整细节 (Proposed Changes)

### 1. `core` 模块
- **[MODIFY]** `crates/core/src/document.rs`
  升级 `DocView` Trait，提供返回 `Cow<'_, str>` 的新方法（例如 `doc_text_in_range` 或升级 `doc_line_text`），封闭底层 `[u8]`，提供纯文本视图。

### 2. `markdown` 模块 (UI 组件层)
- **[MODIFY]** `crates/markdown/src/builder.rs`
  引入类型驱动的状态表示：
  ```rust
  pub enum BlockSource {
      Continuous(std::ops::Range<usize>),
      Fragmented(Vec<std::ops::Range<usize>>),
  }
  ```
  将 `BlockNode` 中的 `text_lines: Vec<String>` 更改为 `source_range: BlockSource`。重构 `build_from_txt` 和 `build_from_novel_doc` 以去除全部 `String` 分配。
- **[MODIFY]** `crates/markdown/src/layout.rs`
  在 `ensure_precise_range` 和 `precise_block_at` 方法的签名中，增加 `doc: &dyn DocView` 作为文本提供源。
- **[MODIFY]** `crates/markdown/src/view.rs`
  修改 `PreviewEngine::render` 以接收并透传 `DocView`。

### 3. `app` 模块
- **[MODIFY]** `crates/app/src/document_view/mod.rs`
  实现升级后的 `DocView` 接口，在提取数据时负责完成 `[u8]` 向 `Cow<'_, str>` 的转换并严格执行 `.expect()` 边界断言。
- **[MODIFY]** `crates/app/src/plugins/markdown.rs` & `editor.rs`
  适配新的 `PreviewEngine::render` 入口，将 App 层的 `DocumentView` 注入渲染管线。

---

## 风险评估与控制 (Risks & Mitigation)

1. **跨间隙拼接风险 (Gap Buffer Spanning)**：
   若一个可视段落恰好横跨底层 TextBuffer 的 Gap，`Cow` 会退化为 `Owned(String)` 而产生拷贝。
   - **应对**：视口内通常文字量极小（不到 10KB），即便发生一次 Gap 内的拼合拷贝，对首屏速度也无足轻重。
2. **多行合并的 Range 映射精度与内存开销**：
   在 Novel 模式中会将多个逻辑行合并为一个自然段。若它们在底层文件中是不连续的，单一 `Range<usize>` 无法表达；若简单使用 `Vec<Range>`，又会带来新的堆分配。
   - **应对**：使用 `BlockSource` 枚举，绝大多数情况走 `Continuous` 零内存分配，仅在出现不连续内容时回退到 `Fragmented`。
3. **UTF-8 边界截断风险**：
   如果 `Range` 的截取位置落在了多字节字符的中间，会导致 `from_utf8` 报错。
   - **应对**：确保 `builder.rs` 在记录段落起始位置时，完全基于合法字符的字符迭代边界计算。当截断发生时，App 层的 `.expect()` 会使其 fail-fast，方便定位。

## User Review Required

> [!IMPORTANT]
> 方案已精简并定型。如果您对该“问题-方案-风险”的文档描述无异议，请回复批准。我将在获得批准后，直接拉取分支/生成 Task 并开始执行底层 AST 重构。
