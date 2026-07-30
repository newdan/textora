# 46b Review Fixes Design

## Goal

修复 `46b08250` 审查确认的事务原子性、Markdown 编辑路由、结构上下文分类和源码行几何问题，同时保持现有公开协议与用户可见编辑行为兼容。

## Scope

本次只闭环当前审查问题，不提前实现统一编辑事务计划中尚未开始的完整 Markdown policy、选区结构保护或性能缓存阶段。

## Design

### 1. App 事务正确性

- `EditPlan::Apply` 使用现有 `DocViewMut::replace_range` / `TextBuffer::replace_range` 完成一次连续替换，不再组合 `delete_selection` 与 `insert_at_cursor`。一次替换只产生一条 Undo 记录和一次 source generation 增量。
- 验证器先检查替换范围的顺序、上界、UTF-8 char boundary 和原文 grapheme boundary；随后构造替换后的最终文本，检查 `cursor_after` 的上界、char boundary 和 grapheme boundary。
- `EditPlan::MoveCursor` 使用同样的最终光标边界验证，非法位置返回结构化错误，不允许底层静默吸附。
- 不增加第三方依赖；最终文本 grapheme 判断复用 `core::unicode::CursorNav` 与 `ReadableDocument for String`。

### 2. Markdown 路由兼容层

- `MarkdownEditorView` 实现 `EditPolicy`，把当前 `EditRequest` 映射到现有 augmenter，并把 `EditAugmentation` 转换为 `EditPlan`。
- 有选区时沿用迁移前行为返回 `UseDefault`；无选区且 augmenter 有结果时返回 `Apply` 或 `MoveCursor`，无结果时返回 `UseDefault`。
- `ViewPlugin::edit_policy()` 返回该实现，使新的 App 事务入口不会绕过列表、引用、表格及空行的既有 Markdown 行为。
- 该适配层是兼容过渡，不引入第二份 Markdown 编辑规则；后续完整 policy 接入时可整体删除。

### 3. Markdown 结构分类

- 用显式结构优先级选择已命中的容器，不能让内层 `Paragraph` 覆盖 `TableCell`、`ListItem` 或 `BlockQuote`。
- 列表复用现有 marker/indent 解析逻辑，保留真实有序序号、任务状态、Tab/空格分隔和 marker/content 范围。
- 引用按当前源码行收集每一级 `> ` marker，并让内容范围从最后一级 marker 后开始。
- 表格在一次 parser 扫描中记录行列 cell 范围，为当前 cell 提供下一行同列起点。
- 标题范围同时覆盖 ATX/Setext、行内 Markdown marker、空标题和 UTF-8 边界；代码 fence 识别长度、尾随空白及 CRLF。
- 空行分类优先信任已经附着的 `SourceLineEntry.role`；未附着时根据 run 是否位于文档首尾或两个渲染块之间推导，首尾空行永远可编辑。

### 4. SourceLineMap 与 View 几何

- `SourceLineAtByte` 保留 `is_blank`，所有构造路径按整行 Unicode whitespace 计算，CRLF、空格与 Tab 空行在 hit-test、光标和视觉导航中保持一致语义。
- `attach_layout` 按半开区间判断重叠：过期渲染段满足 `end <= line.start`。
- 同一源码行对应多个软换行渲染段时，使用首段顶部和末段底部计算完整高度，并将后续空行放在聚合后的底部。

## Error Handling

- 非法事务在修改文档前返回 `EditTransactionError`，文档、selection、generation 和 Undo 栈保持不变。
- 兼容 policy 无法生成 Markdown 增强时返回 `UseDefault`，由 App 统一默认事务处理；不得递归回到旧 App 命令分派。

## Testing

每项生产改动必须先有失败测试并确认失败原因：

- App：单次 generation、单次 Undo、range/cursor UTF-8 与 grapheme 边界、越界 `MoveCursor`、真实 Markdown 空列表 Enter 路由。
- Markdown context：嵌套引用、真实有序序号、Tab marker、表格同列下一行、ATX/Setext/空标题、closing fence、首尾空行。
- Layout/View：CRLF/空白行 cursor 与 hit-test、软换行聚合、半开区间相邻边界。
- 阶段测试通过后运行 `cargo fmt --all -- --check`；最终运行 `./scripts/verify.sh`。

## Non-goals

- 不在本次实现完整的 Markdown 选区结构保护表。
- 不删除旧 augmenter 或旧 WYSIWYG 视觉导航兼容代码。
- 不进行与审查问题无关的模块拆分或性能重构。
