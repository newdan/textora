# WYSIWYG 换行完整性修复实施计划

## 目标

修复 Markdown WYSIWYG 编辑中的以下问题：

1. 退格跨越反斜杠或双空格硬换行时遗留语法标记。
2. 在硬换行视觉边界按 Enter 时产生多余空段并遗留标记。
3. 无选区光标位于 Setext 标题内时，Enter 破坏标题结构。
4. 缩进代码块内按 Enter 后，新行失去代码块所需缩进。
5. 表格纵向跳转落在单元格前导空白之前。
6. 编辑分类器与渲染器使用不同的 pulldown-cmark 解析选项。

生产实现改动限于 `crates/markdown`，不触碰 UI、投影、hit-test 或 app 层；允许同步更新 app 层的跨层回归测试期望，但不改变 app 生产逻辑。

## 修订后的边界决策

### 硬换行反斜杠

连续反斜杠按奇偶性解释：偶数个反斜杠后的换行是软换行；奇数个反斜杠中，
最后一个是硬换行标记，前面的偶数个保留为转义后的正文。Enter 与 Backspace
必须共享这一规则，并覆盖 LF、CRLF 和三个连续反斜杠的测试。

### 缩进代码块空白行

非空代码行按 Enter 时复制当前行的完整前导空白。代码块内部空白行没有可靠的
当前行缩进，因此从同一代码块内最近的非空源码行继承前导空白；优先前一行，
没有前一行时使用后一行。分类结果携带续行前缀，增强阶段不再重新猜测块范围。

### Setext 选区范围

本轮 D4 明确限定为“无选区、单光标”场景：光标位于标题文本或下划线内按
Enter，统一在下划线之后建立可编辑块边界。带选区 Enter 仍由
`selection_augmentation_edit_plan` 管理；跨位置增强无法覆盖选区删除点，保持
`UseDefault`。完整的 Setext 选区语义需要同时设计选区删除、标题保留和光标落点，
作为独立的 `view` 层方案处理，不纳入本轮三文件修复。

## 实施阶段

### 阶段 1：硬换行

- 在 `augmenter.rs` 增加共享的硬换行标记识别工具。
- Backspace 删除换行及其硬换行标记。
- Enter 将已有硬换行边界整体升级为块边界。
- 覆盖双空格、单/三反斜杠、偶数反斜杠、LF 与 CRLF。

### 阶段 2：块上下文

- 增加 `SetextHeading` 分类，并把无选区 Enter 重定向到下划线之后。
- 区分围栏与缩进代码块；缩进代码块分类携带可靠的续行前缀。
- 表格跳转跳过下一单元格的前导空白。

### 阶段 3：解析选项

- 在 `parser.rs` 提取 crate 内共享的 `markdown_options()`。
- `parse_markdown`、`augmenter` 和 `edit_context` 共用该函数。
- pulldown-cmark 自身行为测试可继续显式使用 `Options::all()`。

## 验证

每个阶段执行：

```bash
cargo fmt --all
cargo test -p textora-markdown --lib
```

全部完成后执行：

```bash
cargo clippy --workspace --all-targets -- -D warnings
./scripts/verify.sh
```

最终验收必须包含：硬换行 LF/CRLF/奇偶反斜杠、Setext 无选区、缩进代码块
普通行与内部空白行、表格非空与空单元格、definition list 解析一致性。
