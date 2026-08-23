# Markdown WYSIWYG 结构化换行修复方案

## 目标

彻底修复 WYSIWYG Enter、Shift+Enter 与 Backspace 在 Markdown 物理行、软换行、
硬换行、块边界、行内元素和嵌套容器之间的语义漂移。

## 核心模型

编辑位置不能再只表示为一个互斥块类型。每次结构编辑都必须同时解析：

```text
MarkdownEditPath
├── containers: ListItem / BlockQuote 等外层容器路径
├── leaf: Paragraph / Heading / TableCell / CodeBlock
├── inline_frames: Strong / Emphasis / Link / InlineCode 等行内路径
└── boundary: Text / SoftBreak / HardBreak / BlockBoundary / EditableBlank
```

源码仍是唯一事实源。编辑计划只能生成原子 replacement，不引入 AST 序列化。

## 行为决策

1. 普通 Enter 创建块或当前容器内的下一结构单元。
2. Shift+Enter 创建 Markdown 硬换行，普通叶块使用反斜杠加当前文档换行序列；
   标题、Setext 标题和表格单元格因物理换行会终止当前叶块，改用内联 `<br>`。
3. 在成对行内元素内部拆块时，前块闭合、后块重新打开，文本和样式均保留。
4. 在单 ASCII 空格边界拆块时保留该空格；Enter 后立即 Backspace 必须恢复原文。
5. 容器内生成新物理行时必须保留完整容器前缀；退出空容器时每次只退出一层。
6. 已有续行缩进被替换为结构 marker 时必须消费原缩进，不能重复累积。
7. 表格新增行或退出空行时必须保留外层引用/列表前缀。
8. Backspace 只能合并同一叶块的硬换行，不得跨入新的嵌套块。

## 元素关系矩阵

| 当前元素 | Enter | Shift+Enter | Backspace 合并 |
|---|---|---|---|
| 普通段落 | 生成块边界 | `\\` + 原生换行 | 仅合并同一容器路径 |
| 列表项 | 生成同级 item | 硬换行 + 内容列缩进 | 保留列表归属 |
| 引用 | 生成显式引用行 | 硬换行 + 完整引用前缀 | 前缀路径相同时合并 |
| ATX / Setext 标题 | 后半段成为同容器段落 | `<br>`，标题不终止 | 不跨新叶块合并 |
| 表格单元格 | 移动单元格或新增行 | `<br>`，单元格不终止 | 不跨表格边界合并 |
| fenced / indented code | 普通源码换行 | 普通源码换行 | 按代码块物理行处理 |
| 行内样式 / 链接 / 行内代码 | 闭合后在新块重开 | 闭合、硬换行后重开 | 保留成对 marker |

空列表、空引用每次 Enter 只退出最内层容器；表格新增行和退出空行均继承
外层列表内容缩进或引用 marker。

## 分阶段实施

### 子任务一：内容完整性

- 恢复空格拆分的可逆性。
- 解析光标所在行内元素路径。
- 为可跨块的行内元素生成闭合/重开 marker。
- 覆盖粗体、斜体、删除线、行内代码、链接及嵌套样式。

### 子任务二：容器关系

- 分类器收集完整容器路径，不在 `End(Item)` 提前丢失内层元素。
- 标题、引用、列表、表格共享容器前缀生成逻辑。
- 修复续行缩进重复、嵌套空容器一次退出多层和跨块硬换行合并。

### 子任务三：硬换行输入

- 增加独立的 `InsertLineBreak` 编辑意图。
- app 与 appkit-shell 均保留 Shift 修饰键。
- Markdown 插件生成 `\\` + 原生换行序列；普通文本编辑器回落为普通换行。

## 验证矩阵

每类元素覆盖 LF/CRLF、无选区/选区、Enter/Backspace、普通物理换行、硬换行、
行内样式和至少一层嵌套容器。完成后运行：

```bash
cargo fmt --all
cargo test -p textora-markdown --lib
cargo test -p textora-appkit-shell --lib
cargo test -p textora-app --lib
cargo clippy --workspace --all-targets -- -D warnings
./scripts/verify.sh
```
