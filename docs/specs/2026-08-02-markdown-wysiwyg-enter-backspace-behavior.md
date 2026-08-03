# Markdown WYSIWYG Enter / Backspace 行为规范

## 目标

WYSIWYG 编辑视图把 Enter 解释为“新段落”，而不是源码编辑器的“插入一个换行”。
该语义参考 Typora：普通 Enter 创建段落，Shift+Enter 才表示单行换行。源码换行数量只是实现块边界和可编辑空段的手段，不能直接等同于视觉空行数量。

参考资料：

- [Typora Whitespace and Line Breaks](https://support.typora.io/Line-Break/)
- [Typora Markdown Reference](https://support.typora.io/Markdown-Reference/)

本文中的 `|` 表示光标，不属于源码。

## Enter 行为矩阵

| 场景 | Enter 前 | Enter 后 | 约束 |
|---|---|---|---|
| 段落中部 | `left|right` | `left\n\n|right` | 拆成两个普通段落 |
| 段落末尾 | `paragraph|` | `paragraph\n\n|` | 创建一个尾随空段 |
| 已有软换行前 | `left|\nright` | `left\n\n|\nright` | 创建新段并保留下方原源码行 |
| 已有软换行后 | `left\n|right` | `left\n\n|right` | 在当前点补一个 `\n` |
| 下一块前的段尾 | `paragraph|\n\nnext` | `paragraph\n\n|\nnext` | 只新增一个可编辑空段 |
| ATX 标题中部 | `# left|right` | `# left\n|right` | 前半保留标题，后半成为普通段落；不产生视觉空行 |
| ATX 标题末尾 | `# title|` | `# title\n\n|` | 离开标题并创建普通空段 |
| 标题与下一块仅一个换行 | `# title|\nnext` | `# title\n\n|\nnext` | 必须补两个换行，不能把光标送入 `next` |
| 标题与下一块已有块分隔 | `# title|\n\nnext` | `# title\n\n|\nnext` | 只补一个换行，形成一个可编辑空段 |

## Backspace 行为矩阵

| 场景 | Backspace 前 | Backspace 后 | 约束 |
|---|---|---|---|
| 拆分后第二段开头 | `left\n\n|right` | `left|right` | 删除完整段落边界，不保留不可见软换行 |
| 标题中部分割后的段首 | `# left\n|right` | `# left|right` | 删除单个标题行边界 |
| 尾随空段 | `paragraph\n\n|` | `paragraph|` | Enter 与 Backspace 可逆 |
| 块间可编辑空段 | `first\n\n|\nsecond` | `first|\n\nsecond` | 删除空段，恢复一个隐藏块分隔 |
| 普通软换行后的行首 | `left\n|right` | `left|right` | 删除一个源码换行 |
| 标题后的普通段首 | `# title\n\n|paragraph` | `# title|paragraph` | 删除完整块边界，将段落并入标题 |
| 标题 marker 后 | `### |title` | `|title` | 优先去掉标题样式，不触发跨块合并 |

CRLF 文档中的一个逻辑换行是完整的 `\r\n` 序列。Backspace 删除块边界时必须按逻辑换行删除，不能留下孤立的 `\r`。

## 空行与间距不变量

连续换行位于两个已渲染块之间时：

- `a\n\nb`：一个源码空行，仅作为隐藏块分隔；可编辑空段数为 0。
- `a\n\n\nb`：第一个空行隐藏，第二个空行可编辑；可编辑空段数为 1。
- `a\n\n\n\nb`：第一个空行隐藏，其余两个空行可编辑；可编辑空段数为 2。
- 文档末尾没有“下一块”，所以尾随空行全部可编辑。

段落和标题的常规视觉间距由 Markdown 布局样式负责。隐藏块分隔不能再占一个完整行高；用户显式创建的额外空段必须占完整行高并可放置光标。

## 实现约束

1. Enter 先按 Markdown 块上下文分类，再生成一次原子 replacement。
2. 光标位于已有软换行前时，新段必须插在当前行与原下方行之间，不能把后续输入拼到原下方行开头。
3. 标题解析事件的范围可能包含行尾换行；标题命中范围必须截止到真实标题内容末端。
4. Backspace 在非空普通段落的物理行首，应删除紧邻光标前的完整换行 run。
5. 空行 Backspace、段首 Backspace、单字符删除和 marker 删除必须按此优先级分派，避免 marker 行被误并入上一块。
6. Enter/Backspace 的核心段落与 ATX 标题场景应满足可逆性测试；已有软换行的规范化场景只要求视觉语义正确，不要求逐字节恢复原始换行形式。

## 当前范围

本文固定普通段落、ATX 标题、块间空段及其 Backspace 行为。列表、任务列表、引用和表格继续使用既有专用结构编辑策略；Shift+Enter、Setext 标题源码重写和代码块内部换行不在本次修改范围内。
