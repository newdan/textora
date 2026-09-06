# Markdown WYSIWYG Enter / Backspace 行为规范

## 目标

WYSIWYG 编辑视图把 Enter 解释为“新段落”，而不是源码编辑器的“插入一个换行”。
该语义参考 Typora：普通 Enter 创建段落，Shift+Enter 才表示单行换行。源码换行数量只是实现块边界和可编辑空段的手段，不能直接等同于视觉空行数量。

参考资料：

- [Typora Whitespace and Line Breaks](https://support.typora.io/Line-Break/)
- [Typora Markdown Reference](https://support.typora.io/Markdown-Reference/)

本文中的 `|` 表示光标，不属于源码。

2026-09-06 已按 [统一段落编辑语义](2026-09-06-wysiwyg-paragraph-edit-semantics.md) 修订空段创建、相邻空段删除和段落清空规则。

## Enter 行为矩阵

| 场景 | Enter 前 | Enter 后 | 约束 |
|---|---|---|---|
| 段落中部 | `left|right` | `left\n\n|right` | 拆成两个普通段落 |
| 文首正文起点 | `|paragraph` | `\n|paragraph` | 只前插一个空段 |
| 已有 EOF 空段 | `paragraph|\n` | `paragraph\n\n|\n` | 保留原尾空段，同时新建一段 |
| 段落末尾 | `paragraph|` | `paragraph\n\n|` | 创建一个尾随空段 |
| 已有软换行前 | `left|\nright` | `left\n\n|\nright` | 创建新段并保留下方原源码行 |
| 已有软换行后 | `left\n|right` | `left\n\n|right` | 在当前点补一个 `\n` |
| 下一块前的段尾 | `paragraph|\n\nnext` | `paragraph\n\n|\nnext` | 只新增一个可编辑空段 |
| 标题内容起点 | `# |title` | `\n# |title` | 前插普通空段，原标题与光标内容起点保留 |
| ATX 标题中部 | `# left|right` | `# left\n|right` | 前半保留标题，后半成为普通段落；不产生视觉空行 |
| ATX 标题末尾 | `# title|` | `# title\n\n|` | 离开标题并创建普通空段 |
| 标题与下一块仅一个换行 | `# title|\nnext` | `# title\n\n|\nnext` | 必须补两个换行，不能把光标送入 `next` |
| 标题与下一块已有块分隔 | `# title|\n\nnext` | `# title\n\n|\nnext` | 只补一个换行，形成一个可编辑空段 |
| 段内单空格前 | `left| right` | `left\n\n|right` | 吃掉那一个空格 |
| 段内单空格后 | `left |right` | `left\n\n|right` | 同上 |
| 列表硬换行边界 | `- first\|\n  second` | `- first\n- |second` | 不残留硬换行反斜杠或续行缩进 |
| 列表懒延续换行 | `- item|\npara` | `- item\n- |para` | 替换该换行，不插入额外空行 |
| 引用硬换行边界 | `> first\|\n> second` | `> first\n> |second` | 不残留硬换行反斜杠或重复引用 marker |
| 引用懒延续换行 | `> first|\nsecond` | `> first\n> |second` | 后行补显式 `>` |
| 表格非末行 | 格内 | 源码不变，光标到下一行同列 | 保持既有跳格行为 |
| 表格末行有内容 | `| b |` 内 | 表末多一行 `|  |` | 列数与当前行相同 |
| 表格空表体末行 | 空格内 | 删除该行，表后写入 `\n\n|` | 不删除表头 |
| Setext 标题内容起点 | `|标题\n====` | `\n|标题\n====` | 前插空段，保留标题和下划线 |
| Setext 标题其他内容或下划线位置 | `标题\n====` 内 | 在下划线行尾建立块边界，光标落到新空段 | 保留标题源码，不重写为 ATX；入口 `setext_heading_enter_augmentation`（分类 `EnterContext::SetextHeading`） |
| 嵌套空列表项 | `- a\n  - b\n    - \|` | 当前行替换为父级前缀 `    ` | 每次只退出一层，连续回车逐层退出；入口 `list_item_enter_augmentation` 的 `empty` 分支 |
| 空嵌套容器 | `- > \|`、`> - \|`、`> > \|` | 退出一层（如 `> > ` → `> `） | 每次只退出最内一层空容器 |
| 列表 marker 内部 | `-\| item` | `- \n- |item` | 归一为内容起点回车，不产出 `-\n item` 懒延续残留 |
| 开头围栏行任意位置 | `\`\`\`rust\|` 等 | 行尾插入单个换行，光标进入代码体第一行 | 围栏与 info string 保持完整；入口 `fence_line_enter_augmentation`（分类 `EnterContext::CodeBlockFenceLine`） |
| 闭合围栏行任意位置 | `code\n\`\`\`\|` | 闭合围栏行尾建立块边界，光标落到围栏外新空段 | 退出代码块；未闭合围栏只有开头围栏行情形 |
| 代码体内部 | 代码行内 | 回落默认裸换行（无 augmentation） | 含形似短围栏的代码行 |

## Backspace 行为矩阵

| 场景 | Backspace 前 | Backspace 后 | 约束 |
|---|---|---|---|
| 拆分后第二段开头 | `left\n\n|right` | `left|right` | 删除完整段落边界，不保留不可见软换行 |
| 标题中部分割后的段首 | `# left\n|right` | `# left|right` | 删除单个标题行边界 |
| 尾随空段 | `paragraph\n\n|` | `paragraph|` | Enter 与 Backspace 可逆 |
| 块间可编辑空段 | `first\n\n|\nsecond` | `first|\n\nsecond` | 删除空段，恢复一个隐藏块分隔 |
| 普通软换行后的行首 | `left\n|right` | `left|right` | 删除一个源码换行 |
| 标题后的普通段首 | `# title\n\n|paragraph` | `# title|paragraph` | 删除完整块边界，将段落并入标题 |
| 标题 marker 后（前方没有可编辑空段） | `### |title` | `|title` | 去掉标题样式 |
| 非空段首前有多个空段 | `a\n\n\n\n|b` | `a\n\n\n|b` | 只移除最近一段，不跨空段合并 |
| 标题内容起点前有空段 | `\n# |title` | `# |title` | 先删前方空段，保留标题样式 |
| 列表硬换行下行首 | `- first\\\n  |second` | `- first|second` | 先于 marker 删除 |
| 引用硬换行下行首 | `> first\\\n> |second` | `> first|second` | 先于去掉 `>` |
| 硬换行跨新列表项 | `- first\\\n- |second` | 不合并两项 | 交给既有 marker 或默认路径 |

CRLF 文档中的一个逻辑换行是完整的 `\r\n` 序列。Backspace 删除块边界时必须按逻辑换行删除，不能留下孤立的 `\r`。

## Delete（前向删除）行为矩阵

实现入口：`augment_delete_forward`。先按 `EditableParagraphMap` 删除最近可编辑空段；保留内联换行处理，最后一个 grapheme 委托统一范围清空，剩余块边界交由既有合并/原子块护栏。

| 场景 | Delete 前 | Delete 后 | 约束 |
|---|---|---|---|
| 段末、下一行是普通段落 | `a|\n\nb` | `a|b` | 没有可编辑空段时合并两段；与段首 Backspace 对称 |
| 段末、前方有多个空段 | `a|\n\n\n\nb` | `a|\n\n\nb` | 只删最近一段，后方块类型不影响该优先级 |
| 段末、仅有一个 EOF 空段 | `a|\n` 或 `a|\n\n` | `a|` | 直接移除该视觉空段 |
| 段末、下一行自成独立块（没有可编辑空段） | `文字|\n\n---` | 不变 | 消费型空操作（不删字节），拦截并线破坏结构（ATX/围栏/HR/列表 marker/setext 下划线 `===`/引用/表格行） |
| 闭合围栏行尾紧贴下一段 | ` ``` |\npara` | 不变 | 仅边界恰好一个换行序列时拦截；有空行兜底（≥2 个换行）交回默认删除 |
| 代码体内、下一行是闭合围栏 | 代码行尾 | 不变 | 代码体内只有闭合围栏行受保护，普通代码行允许默认逐字符删除 |
| 行尾硬换行标记前 | `first\||\nsecond` | 默认逐字符删除 | 硬换行标记属段内结构，护栏不介入 |

## InsertText（块间空行输入）行为

实现入口：`augment_insert_text`。使用 `EditableParagraphMap` 判定当前空段及容器，仅补齐该段成为非空正文所需的左右隐藏分隔，保留其余已有空段、源码缩进和引用前缀。空格、Tab 和多行输入继续对应默认路径。

Backspace、Delete 与选区删除清空同一个完整正文段落时共用 `erase_range`：依据解析器完整段落范围撤除内容及其样式语法，仅移除该段右侧同容器内多余的隐藏分隔，保留一个可编辑空段。部分内容、跨段或跨原子块选区保留默认删除语义。

## 空行与间距不变量

连续换行位于两个已渲染块之间时：

- `a\n\nb`：一个源码空行，仅作为隐藏块分隔；可编辑空段数为 0。
- `a\n\n\nb`：第一个空行隐藏，第二个空行可编辑；可编辑空段数为 1。
- `a\n\n\n\nb`：第一个空行隐藏，其余两个空行可编辑；可编辑空段数为 2。
- 文末 `a\n` 与 `a\n\n` 都有一个可编辑空段；多行尾空白隐藏一个结构分隔，其余各占一段。单次操作以段数为准，Undo/Redo 恢复精确字节。

段落和标题的常规视觉间距由 Markdown 布局样式负责。隐藏块分隔不能再占一个完整行高；用户显式创建的额外空段必须占完整行高并可放置光标。

## 实现约束

1. Enter 先按 Markdown 块上下文分类，再生成一次原子 replacement。
2. 光标位于已有软换行前时，新段必须插在当前行与原下方行之间，不能把后续输入拼到原下方行开头。
3. 标题解析事件的范围可能包含行尾换行；标题命中范围必须截止到真实标题内容末端。
4. 段首 Backspace 与段尾 Delete 优先删除最近的可编辑空段；没有空段时才合并完整结构边界。
5. 空行 Backspace、段首 Backspace、单字符删除和 marker 删除必须按此优先级分派，避免 marker 行被误并入上一块。
6. Enter/Backspace 的核心段落与 ATX 标题场景应满足可逆性测试；已有软换行的规范化场景只要求视觉语义正确，不要求逐字节恢复原始换行形式。
7. 在单个 ASCII 空格边界拆段时，Enter 会消费该空格；后续 Backspace 只合并块边界，不恢复已消费的空格。
8. 列表和引用必须复用段落的硬换行识别规则，包括反斜杠奇偶性、至少两个行尾空格以及 LF/CRLF 边界。
9. 表格末行 Enter 必须产生源码变更：有内容时按当前列数新增空行，空表体行时删除该行并退出表格。
10. 光标落在列表 marker 内部时，Enter 归一到内容起点处理，不产出懒延续残留。
11. 嵌套空列表项与空嵌套容器的 Enter 每次只退出最内一层，不得一次退到顶层。
12. 围栏行（开头/闭合）Enter 的插入点恒为该行行尾，不得在围栏中间拆行。
13. Delete 在没有可编辑空段的块边界使用护栏；有空段时先删一段，并保留容器退出所需分隔。

## 当前范围

本文固定普通段落、ATX 标题、Setext 标题（保留下划线源码、在其后建块边界）、列表项、任务列表项、引用行、表格单元格、围栏代码块围栏行、块间空段的 Enter/Backspace 行为，以及 Delete 块边界护栏、空段输入与方向无关的段落清空。Setext 标题由 `EnterContext::SetextHeading` 专门分流，已在范围内。列表项内新开段落、把普通段落改写为 Setext 源码形态仍不在范围内；代码体内部继续使用默认裸换行策略。
