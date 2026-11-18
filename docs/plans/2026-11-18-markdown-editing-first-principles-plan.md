# Markdown WYSIWYG 编辑路径第一性原理修复计划

> **执行原则**：沟通先行、TDD（先写失败测试再改代码）、每个子任务最多修改 **3 个文件**、每次子任务完成后确保 `cargo test -p textora-markdown` 编译与测试通过，全部阶段完成后执行 `./scripts/verify.sh` 全量验证。

## 一、 核心目标与第一性原理不变量

### 1. 核心目标
消除 Markdown WYSIWYG 编辑路径中**物理源码行（Physical Line）**、**AST 逻辑块/逻辑行（Logical Block & `ProjectedText`）**、**视觉折行（Visual Wrapped `FlatLine`）**与**编辑增强器（`augmenter` / `commands`）**四层空间之间的语义漂移与结构坍塌。

### 2. 四大第一性原理不变量（Invariants）

1. **单一结构真相源（Single Source of Truth）**：
   - 容器归属（`BlockQuote` / `ListItem` 嵌套路径）与叶块边界必须由 AST 事件栈或 `MarkdownDoc` + `EditableParagraphMap` 统一决定，禁止用单物理行 `locate_source_line_bounds` 盲猜跨行容器状态。
2. **换行算子语义正交（Orthogonal Linebreak Semantics）**：
   - **`Enter` = 逻辑结构递进**：在段落中拆段（保留 `left|\nright` -> `left\n\n|\nright` 与 `left\n|right` -> `left\n\n|right` 既有落点契约）；在 `BlockQuote` 行中拆段（`\n>\n> `），行尾插 `\n> ` 形成可见空段；在 `ListItem` 中新建同级项；在空容器行上逐层退出最内层容器；在 ATX `Heading` 中部拆分为标题 + 同容器段落；Setext 标题保持起点前插空段、其余位置从下划线后退出。
   - **`Shift+Enter` = 块内硬换行（`HardBreak`）**：流式叶块（`Paragraph` / `ListItem` / `BlockQuote`）写 `\{newline}{container_prefix}`；单行受限块（`Heading` / `TableCell`）写 `<br>`；字面量块（`CodeBlock` / `Metadata` / `HtmlBlock`）写字面换行。
   - **`SoftBreak` = 视觉同段空格**：段内单 `\n`（及后续被折叠的续行空白）在 `Backspace` / `Delete` 删除时视为单一折叠区间（Collapsed Span），一并清理折叠空白（不留多余空格，与 `left\n|right` -> `left|right` 对齐）；非末行行尾敲空格必须有光标视觉补偿。
3. **跨行投影单调不减（Monotonic Non-Decreasing Projection）**：
   - 任何跨 `HardBreak` / `<br>` 的行内样式（`StyleSpan`），在单条逻辑行物化展开（`materialize_projected_line`）时，其 `source_range` 投影不得越过当前逻辑行的源码边界，所有边界锚点必须保持源码字节**单调不减（`next.byte >= prev.byte`）**，兼容共享同一源码字节的 IME 虚拟预编辑字符及折叠范围。
4. **成对定界符与二维结构原子性（Structural Integrity on Selection & Paste）**：
   - 选区编辑全分支（删除 / 打字 / 粘贴 / 回车）、多行粘贴与块级转换命令（`commands`）不得单边破坏成对块级定界符（如代码块围栏 ```` ``` ````、Setext 底线 `===`/`---`、表格管道与分隔行 `| --- |`），且支持单次 Undo 恢复原文。

---

## 二、 物理换行、逻辑换行与 Markdown 元素关系矩阵

| 当前元素 / 上下文 | 源码物理形态 | 视觉/逻辑形态 | `Enter` 行为 | `Shift+Enter` 行为 | `Backspace` / `Delete` 边界行为 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **顶层段落 (`Paragraph`)** | 块间 `\n\n`；段内 `\n` (`SoftBreak`) | 块间间距；`SoftBreak` 折叠为 `' '` | 段落中拆 `\n\n`；保留 `left\|\nright`->`left\n\n\|\nright` 与 `left\n\|right`->`left\n\n\|right` | 插入 `\\\n` 产生块内新逻辑行 | 合并段落或删除相邻可编辑空段；在 `SoftBreak` 处清理完整折叠区间（含续行前导空白） |
| **引用内段落 (`BlockQuote`)** | 每行 `> ` 或懒延续行；段间 `>\n>` | 引用容器内段落 | **行中拆段**：`\n>\n> `；**行尾**：`\n> `（形成空段，再 Enter 退出或输入文字自动补全）；空行退一层 | 插入 `\\\n> ` 产生引用内硬换行 | 段首退格合并回前段或剥离当前行 `> `；懒延续行显式化 |
| **列表项 (`ListItem`)** | `- ` / `1. ` / `- [ ] ` + 续行缩进 | 列表项 + 子块 | 非空拆为同级新项 `\n{indent}{marker}`；空项退出一层列表 | 插入 `\\\n{content_prefix}` 留在当前项内 | 项首退格剥离 marker；`Tab`/`Shift+Tab` 按前序兄弟内容列调整完整子树缩进 |
| **ATX 标题 (`# Heading`)** | 单物理行 `# text` | 标题行（点击正文保持 `# ` 折叠；光标入前缀展开） | 首部前插空段；中部 `\n{prefix}` 拆出正文段；尾部建空段 | 插入 `<br>` 块内折行 | `# ` 前退格消费；正文首退格剥离 `# ` 转为段落；AST 首个顶层 H1 防降级 |
| **Setext 标题 (`Title\n===`)** | 两物理行（正文 + `===`/`---`） | 标题行（底线折叠隐藏） | 起点前插空段；其余位置在下划线后建块边界退出 | 插入 `<br>` 块内折行 | **删光标题正文时同步移除底线**，防止突变为 `HorizontalRule` |
| **围栏代码块 (`CodeBlock`)** | ```` ```lang\n...\n``` ```` | 激活态显示围栏行，非激活态隐藏围栏 | 开头围栏行尾进代码体；代码体内插 `\n{prefix}`；闭合围栏后退块 | 同 `Enter`（字面换行） | **保护开闭围栏完整性**：空块打字落入代码体，不破坏围栏；允许编辑语言名 |
| **表格单元格 (`TableCell`)** | `\| cell \|` 单物理行 | 二维网格布局（支持单元格内软折行） | 跳至下一行同列；末行空行退出表格，非空新增行 | 插入 `<br>` 单元格内换行 | `Up`/`Down` 按**同列上下视觉行**导航；跨单元格选区全分支（删/打/贴/回车）保 `\|` 与分隔行 |

---

## 三、 分阶段实施计划（6 阶段 / 13 子任务，每任务 ≤ 3 文件）

### Phase 1：容器与 `SoftBreak` 的换行语义收敛

#### Task 1.1：统一 `BlockQuote` 段中拆段与混合容器分类优先级
- **涉及文件（3 个）**：
  1. `crates/markdown/src/augmenter/classify.rs`
  2. `crates/markdown/src/augmenter/enter_rules.rs`
  3. `crates/markdown/src/augmenter/augmenter_tests.rs`
- **具体修改**：
  1. **修复引用懒延续行与混合容器分类**：在 `classify_enter_context` 中记录列表项 `quote_depth` 与活跃 `BlockQuote` 的前缀路径：
     - 使 `> hello\nworld` 的懒延续行 `world` 正确归属 `BlockQuoteLine`；
     - 当同时命中 `BlockQuote` 与 `ListItem` 时，仅当当前行引用深度严格大于列表项起始 `quote_depth`（如 `- > quote`）才归属 `BlockQuoteLine`；若相等（如 `> - para 1\n>\n>   para 2`）则优先归属内层 `ListItem`（保留列表缩进）。
  2. **实现 `BlockQuote` 段中 `Enter` 拆段**：`blockquote_enter_augmentation` 在非空引用段落**行中**（非段尾/行尾）按 `Enter` 时，插入双行段落分隔 `{newline}{continuation_prefix}{newline}{continuation_prefix}`，将当前引用段拆为两段；在行尾（下一行无同级引用内容）仍插单行 `{newline}{continuation_prefix}`（由 `EditableParagraphMap` 保持可编辑空行/退出语义）。
- **验证命令**：`cargo test -p textora-markdown --lib augmenter`

#### Task 1.2：保留 `SoftBreak` 既有 `Enter` 落点，修复折叠前导空白删除与行尾空格几何补偿
- **涉及文件（3 个）**：
  1. `crates/markdown/src/augmenter/primitives.rs`
  2. `crates/markdown/src/augmenter/backspace.rs`
  3. `crates/markdown/src/view/engine_caret.rs`
- **具体修改**：
  1. **保留既有 `Enter` 落点**：严格保留 `left|\nright` -> `left\n\n|\nright` 与 `left\n|right` -> `left\n\n|right` 现有规范行为与测试。
  2. **清理 `SoftBreak` 折叠空白**：`backspace_paragraph_boundary`（如 `left\n   |right`）与 `delete_forward_block_boundary`（如 `left|\n   right`）识别段内 `SoftBreak` 折叠前导空格/Tab，删除时将 `\n` 连同被折叠前导空白一并清除（得到 `left|right`），不残留多余空格。
  3. **修复 `SoftBreak` 前敲空格光标冻结**：`engine_caret.rs` 的 `trailing_stripped_space_advance` 将检查基准扩展至当前物理行尾（`SoftBreak` 之前被 parser 剥离的尾随空格），使多物理行段落非末行敲空格时光标正常右移。
- **验证命令**：`cargo test -p textora-markdown --lib`

---

## Phase 2：行内元素（Inline Spans）跨行投影与语法死区修复

#### Task 2.1：修复跨 `HardBreak` / `<br>` 行内样式的边界展开，保证源码字节单调不减
- **涉及文件（3 个）**：
  1. `crates/markdown/src/builder.rs`
  2. `crates/markdown/src/edit.rs`
  3. `crates/markdown/src/edit_tests.rs`
- **具体修改**：
  1. **更新前序逻辑行闭合范围**：`builder.rs::finalize_inline_source_range` 闭合行内样式时，同步更新同一 `BlockNode` 内已提交的前序逻辑行（跨 `HardBreak` / `<br>`）的 `StyleSpan::source_range.end`。
  2. **按本行归属展开边界**：在 `edit.rs::inline_expansion` 中，对于跨逻辑行的 `StyleSpan`（如 `**bold\\\ntext**` 或 `**bold<br>text**`），仅当开定界符位于当前逻辑行内时才展开 `prefix`，仅当闭定界符位于当前逻辑行内时才展开 `suffix`。
  3. **验收标准**：同时验证各逻辑行只展开本行定界符、跨行拼接后边界满足源码字节**单调不减（`>=`）**、且触发 IME 预编辑（`Virtual` 锚点共享同一源码字节）时仍能正确构建 `SourceProjectionIndex` 并定位光标。
- **验证命令**：`cargo test -p textora-markdown --lib edit`

#### Task 2.2：支持嵌套行内样式外层标记保留、顶层 `HtmlBlock` 保真与 `Image` 源码展开
- **涉及文件（3 个）**：
  1. `crates/markdown/src/builder.rs`
  2. `crates/markdown/src/edit.rs`
  3. `crates/markdown/src/builder_tests.rs`
- **具体修改**：
  1. **顶层 `HtmlBlock` 保真**：在 `builder.rs` 中将根 `Container` 上由顶层 `Event::Html` 产生的 `pending_line` 打包为 `BlockNode` 推入文档块列表，防止在 `flush_line()` 时静默蒸发。
  2. **图片 `![alt](url)` 源码展开**：记录图片的 `StyleSpan` 并允许光标移入时展开 `![alt](url)` 原始源码进行编辑。
  3. **嵌套行内样式外层标记保留**：修复嵌套行内样式（如 `**[link](url)**`、`***text***`）外层帧的展开与范围闭合。
- **验证命令**：`cargo test -p textora-markdown --lib builder`

---

### Phase 3：Setext 标题删空归一化与代码块围栏边界护栏

#### Task 3.1：Setext 标题删空归一化（保留 ATX 正文点击折叠与 Setext 回车既有规范）
- **涉及文件（3 个）**：
  1. `crates/markdown/src/augmenter/editable_paragraph_edit.rs`
  2. `crates/markdown/src/augmenter/editable_paragraph_erasure_tests.rs`
  3. `crates/markdown/src/view/view_heading_container_tests.rs`
- **具体修改**：
  1. 严格保留 `heading_marker_at_cursor` 的正文点击折叠行为（`heading_content_click_keeps_prefix_hidden_and_caret_stable`）与 Setext 标题 `Enter` 既有规范（起点前插空段，其余位置从下划线后退出）。
  2. 在 `editable_paragraph_edit::erased_paragraph` 中覆盖 Setext `BlockKind::Heading`：当删除范围清空了 Setext 标题全部可见内容（如 `"A\n---"` 删掉 `'A'`）时，同步删除下一行底线 `---` / `===`，防止突变为 `HorizontalRule` 或残留 `"==="`。
- **验证命令**：`cargo test -p textora-markdown --lib editable_paragraph`

#### Task 3.2：围栏代码块（`CodeBlock`）空块输入落点与围栏完整性保护
- **涉及文件（3 个）**：
  1. `crates/markdown/src/augmenter/mod.rs`
  2. `crates/markdown/src/augmenter/backspace.rs`
  3. `crates/markdown/src/augmenter/augmenter_tests.rs`
- **具体修改**：
  1. **空围栏代码块输入保护**：光标位于空围栏代码块 ```` ```\n``` ```` 的闭合围栏行首并触发 `InsertText` 时，自动在闭合围栏行前插入 `{text}{newline}{continuation_prefix}`，将文字写入代码体，防止拼出 ```` x``` ```` 吞噬后续文档。
  2. **围栏行 `Backspace` / `Delete` 精细保护**：允许在开头围栏行正常增删 `info string` 语言名，仅拦截破坏围栏字符（`< MIN_FENCE_MARKER_LENGTH` 或短于配对围栏）的删除操作；保护闭合围栏行不被删坏。
- **验证命令**：`cargo test -p textora-markdown --lib augmenter`

---

### Phase 4：列表项子树缩进、有序列表重编号与块级命令重构

#### Task 4.1a：按前序兄弟内容列调整完整列表项及其子树的 `Tab` / `Shift+Tab` 缩进
- **涉及文件（3 个）**：
  1. `crates/markdown/src/augmenter/mod.rs`
  2. `crates/markdown/src/augmenter/source_slice.rs`
  3. `crates/markdown/src/augmenter/augmenter_tests.rs`
- **具体修改**：
  1. **完整列表项子树缩进（`Tab`）**：通过解析器定位当前 `ListItem` 及其全部子树行范围，复用 `list_item_content_prefix` 计算前序同级兄弟列表项的内容起始列宽度：
     - 无前序同级兄弟项时消费 `Tab`（阻止插入 4 空格误成 `IndentedCodeBlock`）；
     - 有前序兄弟项时，为当前列表项及其续段、子列表的全部物理行按目标列差插入前导空格，使重解析后成为前序兄弟项的子列表。
  2. **完整列表项子树反缩进（`Shift+Tab`）**：处于嵌套列表中时，按父项内容列宽度整体剥离当前项及其子树的缩进空格，提升为父项的同级后继项。
  3. **验收测试**：校验重解析后的 AST 父子关系，覆盖无前一兄弟、多位有序编号（`10. `）、带续段和子列表的列表项。
- **验证命令**：`cargo test -p textora-markdown --lib augmenter`

#### Task 4.1b：有序列表插入/退出时的同级源码自动重编号
- **涉及文件（2 个）**：
  1. `crates/markdown/src/augmenter/enter_rules.rs`
  2. `crates/markdown/src/augmenter/augmenter_tests.rs`
- **具体修改**：
  1. 当在有序列表项中按 `Enter` 插入新项（或空项退出导致后续列表项接续前序号）时，同步重排同一列表下紧随其后的同级有序列表项源码编号，保留 `.` / `)` 分隔符格式，消除光标进出时的编号跳变。
- **验证命令**：`cargo test -p textora-markdown --lib augmenter`

#### Task 4.2：块级命令（`commands`）AST 首个 H1 识别、Setext 转换与列表前缀互转
- **涉及文件（2 个）**：
  1. `crates/markdown/src/commands/mod.rs`
  2. `crates/markdown/src/commands/commands_tests.rs`
- **具体修改**：
  1. **用 AST 排除伪 H1 并保留首个 H1 保护**：将 `first_h1_range` 改为基于 `parse_markdown` 查找真正的首个顶层 ATX H1（排除围栏代码块内的 `# comment`），保留 `heading_command_does_not_demote_the_notora_title_h1` 的防降级保护。
  2. **Setext 标题级别转换与多物理行块对齐**：`plan_heading` 转换 Setext 标题时同步清除其下划线行；`plan_line_prefix` 在含 `SoftBreak` 的多物理行普通段落内执行时覆盖整个逻辑段落的所有物理行。
  3. **列表前缀互转**：`has_line_prefix` / `remove_line_prefix` 完整识别无序、有序、任务列表前缀；在三者之间切换时先剥离旧前缀再写入目标前缀，防止叠加套娃或残留 `[ ] `。
- **验证命令**：`cargo test -p textora-markdown --lib commands`

---

### Phase 5：多行粘贴、跨块选区全分支保护与表格二维导航

#### Task 5.1：多行文本粘贴（`InsertText`）接入空段物化与容器前缀续写
- **涉及文件（3 个）**：
  1. `crates/markdown/src/augmenter/editable_paragraph_edit.rs`
  2. `crates/markdown/src/augmenter/mod.rs`
  3. `crates/markdown/src/augmenter/augmenter_tests.rs`
- **具体修改**：
  1. **可编辑空段多行粘贴**：移除 `editable_paragraph_edit::insert_text` 对换行符的放行，确保在可编辑空段粘贴多行文本时同样补齐首尾隐藏分隔及容器前缀，防止与下方块粘连。
  2. **容器内多行粘贴**：在 `BlockQuote` / `ListItem` 内粘贴多行文本时，为第 `2..N` 行自动补齐容器 `continuation_prefix`。
- **验证命令**：`cargo test -p textora-markdown --lib augmenter`

#### Task 5.2：选区替换全分支（删除 / 打字 / 粘贴 / 回车）的结构保护
- **涉及文件（3 个）**：
  1. `crates/markdown/src/view/editor_view.rs`
  2. `crates/markdown/src/augmenter/backspace.rs`
  3. `crates/markdown/src/view/view_paragraph_semantics_tests.rs`
- **具体修改**：
  1. 在原始源码上统一计算选区允许删除/替换的结构安全计划，并覆盖 `plan_selection_edit` 的全部三个入口（**删除、打字/粘贴、回车/换行**），将安全删除与后续输入/回车合并为单个原子 `EditTransaction`（确保一次 Undo 完整还原）：
     - **跨围栏代码块保护**：选区单边跨越围栏时，全包则扩至完整开闭围栏，部分跨入则保留围栏结构、仅修改允许区域，严禁留下孤立闭合围栏。
     - **跨表格单元格保护**：选区跨越同一 `Table` 多个单元格时，按单元格裁剪清空选中内容、保留 `|` 与分隔行 `| --- |`（打字/粘贴写入首个选中单元格并做单行/管道符安全转义，回车跳入下一行同列）。
     - **Setext 护栏**：跨块选区替换后若段落紧贴 `---` / `===`，自动保留空行分隔防止突变 Setext 标题。
  2. **验收测试**：用同一组跨围栏、跨单元格选区分别验证删除、打字、粘贴、回车，并断言单次替换事务可完整 Undo 恢复原文。
- **验证命令**：`cargo test -p textora-markdown --lib view_paragraph_semantics_tests`

#### Task 5.3：表格（`Table`）内的二维垂直方向键导航（`Up` / `Down`）
- **涉及文件（2 个）**：
  1. `crates/markdown/src/view/engine_navigation.rs`
  2. `crates/markdown/src/view/view_tests.rs`
- **具体修改**：
  1. 在 `engine_navigation.rs::visual_move_in_projection_sequence` 中，针对 `ProjectionOwnerId::TableCell { table_start, row, column, .. }` 按同列（`column` 相同、相邻 `row`）查找目标视觉行，解决 `Up`/`Down` 在同行相邻列横跳问题。
- **验证命令**：`cargo test -p textora-markdown --lib view_tests`

---

### Phase 6：规范同步与全工作区综合验证

#### Task 6.1：更新行为规范文档与端到端回归测试，执行 `./scripts/verify.sh`
- **涉及文件（2 个）**：
  1. `docs/specs/2026-08-02-markdown-wysiwyg-enter-backspace-behavior.md`
  2. `crates/markdown/tests/wysiwyg_chain_regressions.rs`
- **具体修改**：
  1. 在 `2026-08-02-markdown-wysiwyg-enter-backspace-behavior.md` 中更新引用块行中 `Enter` 拆段（`\n>\n> `）、`SoftBreak` 折叠空白清理等行为规范。
  2. 在 `wysiwyg_chain_regressions.rs` 中补充端到端连续编辑回归测试。
  3. 运行 `cargo fmt --all` 与 `./scripts/verify.sh` 完成最终全量验收。

---

## 四、 已确认的设计决策

1. **`BlockQuote` 内 `Enter` 与 `Shift+Enter` 分工（新决策）**：`BlockQuote` 行中按 `Enter` 统一拆段（插入 `\n>\n> `，推翻 2026-08-22 仅插单行 `\n> ` 的旧约定）；行尾按 `Enter` 插入 `\n> ` 形成可见空行以便二次 `Enter` 退出或继续输入；`Shift+Enter` 作为引用内硬换行（插入 `\\\n> `）。
2. **`SoftBreak` 既往落点与 ATX/H1 保护（保留历史决策）**：保留 `left|\nright` -> `left\n\n|\nright` 与 `left\n|right` -> `left\n\n|right`；保留 ATX 正文点击保持 `# ` 折叠；保留 AST 首个顶层 H1 防降级保护；保留 Setext 起点外下划线后退出。
3. **`ListItem` 的 `Tab` / `Shift+Tab` 缩进宽度**：严格按前序同级兄弟列表项的内容起始列宽度（复用 `list_item_content_prefix`）移动完整列表项及其子树，无前序兄弟项时不产生非法缩进。
