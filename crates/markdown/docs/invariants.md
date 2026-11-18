# 不变量、红线与常见踩坑点

修改本 crate 前请先核对以下约束。

## 1. 架构与跨层红线

1. **禁止向上游反向依赖**：`textora-markdown` 严禁依赖 `app` 或 `notora-app`；同时 `ui`、`appkit-core`、`appkit-shell`、`notora-core` 也严禁反向依赖 `textora-markdown`（CI 中 `scripts/check_architecture.sh` 会阻断）。
2. **源码是唯一持久化事实源**：Markdown 编辑和 MMF 思维导图都不维护独立的可变文档树。所有编辑命令、按键增强、导图拖拽最终都必须输出源码字节替换事务（`EditAugmentation` / `SemanticEditPlan` / `EditPlan`），经由宿主提交后再重新解析与布局。
3. **禁止用局部几何补丁掩盖边界不一致**：不要为特定元素（如空标题、分割线）单独写硬编码的光标 Y 偏移或假空格；几何必须来自真实的 `LaidOutBlock` / `FlatLine` 与统一的 `SourceProjectionIndex`。

## 2. 坐标与字节安全

1. **字节偏移必须落在 UTF-8 字符边界**：所有外部传入的 `cursor_byte` / `range` 必须校验 `is_char_boundary`（或使用 `lib.rs` 的 `safe_byte_idx`）；禁止按 byte 长度做切片假设。
2. **光标准则用 Grapheme，不用 char 或 byte**：UI 交互的水平移动、点击命中、选区边界必须通过 `VisualGraphemeMap` / `SourceProjectionIndex` 对齐到 UAX#29 字素簇，避免把多码点 emoji 或组合音标拆断。
3. **折叠与展开后的投影一致性**：`FlatLine` 的 `requires_source_projection` 为 true 时，必须携带有效的 `VisualLineProjection`；搜索高亮（`view_source_search.rs`）、选区与光标查询必须共用同一套投影，不能各算一套偏移。

## 3. 编辑增强（`augmenter/`）与空段落不变量

每次 `augment_edit` 成功返回 `EditAugmentation` 时必须满足（代码内有 `debug_assert!`）：
- `replace_range.end <= source.len()` 且两端在合法字符边界。
- `cursor_byte_after` 位于替换后字符串的合法字节边界。
- `insert_text` 与 `replace_range` 至少存在其一。

**常见踩坑点**：
- **不要假设换行符只有 `\n`**：源码可能使用 `\r\n`（CRLF），生成新行或计算分隔范围时需保留原有换行风格（`preferred_newline_sequence`）。
- **不要把“插两个 `\n` / 删整个换行串”当通用公式**：必须通过 `EditableParagraphMap` 区分“隐藏的语法分隔”与“可编辑空段落”。删除内容使段落变空时，须与 Backspace/Delete 共用空段擦除逻辑清理多余分隔；块边界合并时须从映射取完整分隔范围（含带空格/Tab 的空行）。
- **闭合 ATX 标题（如 `# Title ##`）与尾空白**：判断光标是否在标题末尾时，需使用有效可见 span 端点而非完整字节末尾，避免在 `##` 前回车拆出新的空标题；标题行尾的两个空格不能被误判为段落硬换行。
- **段首空格**：普通段落行首的字面空格（`paragraph_leading_spaces.rs`）需要保留，但不能触发 Markdown 的 4 空格缩进代码块语义。
- **段落拆分历史（`view_split_history.rs`）**：已确认的段落拆分在边界未变前保留其源码分隔符，避免连续编辑/撤销时分隔语义漂移。

## 4. 布局与增量复用不变量

1. **`flatten_blocks` 与 `layout_block` 展开基数必须一致**：`layout/mod.rs` 中的 `count_laid_out_blocks` / `flatten_blocks` 与 `layout/types.rs` 中的 `expands_into_multiple_laid_out_blocks` 必须镜像 `layout_block` 对 `Container`、`TableRow_`、`TableCell_` 的展开规则。
2. **`y_delta` 增量传播规则**：修改第 `i` 个块的高度后，只应偏移 `i+1` 及之后的块（`apply_deltas` 语义），`LazyLayout` 的单块重排与全量冷启动结果必须一致。
3. **`SourceLineMap` 与 `EditableParagraphMap` 分工**：`SourceLineMap` 负责源码行边界和隐藏分隔折叠；可编辑空段落的视觉几何完全由注入后的 `BlockNode` 正常布局产生，不要在 `LazyLayout` 中重新引入独立的空行高度反推。
4. **代码高亮偏移**：`HighlightSpan` 的 `start` 与 `len` 均为**字节偏移**（匹配 `lsh` 运行时），不是字符下标。

## 5. 智能粘贴与嵌入渲染不变量

1. **粘贴内容优先原则**：当剪贴板同时存在 `plain` 时，HTML/RTF 转换出的 Markdown 必须经过 `paste::selection` 的可见文字等价性校验；一旦出现文字丢失、重复或乱序，**必须回退纯文本**。转换器不得发起网络请求，也不保留本地/内嵌二进制图片（`data:`/`file:`/`cid:`）。
2. **公式与 Mermaid 容错**：解析或光栅化失败、超出引擎尺寸预算时，必须退回可读源码，不能吞掉用户输入或 panic；`rich-markdown` feature 关闭时，相关代码必须能无报错编译并通过 `lite_tests`。
3. **MMF 导图结构合法性**：标题层级跳跃（如 `#` 直接跳 `###`）、空文档或多根节点属于合法性错误（`MmfDiagnostic`），`MindmapView` 进入 `Invalid` 状态时需安全降级，不能 panic。
