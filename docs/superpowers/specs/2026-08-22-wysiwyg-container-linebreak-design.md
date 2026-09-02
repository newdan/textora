# WYSIWYG 容器换行与块邻接设计

日期：2026-08-22

## 背景

8 月 19 日已修好顶层段落的硬换行 Enter/Backspace、Setext Enter、缩进代码续前缀、表格跨行跳过前导空白，以及分类器与渲染器共用 `markdown_options()`。复审确认这些仍然成立。

剩余问题集中在：**列表/引用等容器没有接到同一套换行原语**，以及 **Enter 遇到已有物理换行时是插入还是替换**。详见 2026-08-22 换行复审。

## 目标

让列表项、引用行与顶层段落共享同一套「硬换行识别 + 结构分隔」规则：

1. 已有硬换行上的 Enter 消耗标记，升级为该容器的结构分隔，不把 `\` 或双空格留给下一行。
2. 硬换行下一视觉行行首 Backspace 合并两行并删掉标记，优先于删除 `>` / 列表 marker。
3. 光标落在容器内已有换行上时：若下一行是懒延续（没有结构标记），用结构分隔**替换**该换行；若下一行已经带 `>` 或列表 marker，则插入一条新的结构行，不得把标记写进下一行开头。
4. 在空格处拆段时吃掉那一个 ASCII 空格，避免新段前导空格或旧段尾随空格。
5. 表格末行 Enter 不再是空操作：有内容则新增一行，空行则退出表格。

## 非目标（本轮不做）

- Shift+Enter / `InsertLineBreak`。没有跨层输入映射，也不做硬换行的可见提示。用户暂时仍不能**创建**硬换行，只能正确编辑已有硬换行。
- `<br>` 渲染、HTML 块进树。这是 parser/builder 问题，与 augmenter 换行正交。
- `SourceLineMap` 的 dummy `Paragraph` 角色、hit-test、块间样式间距统一。那是布局层邻接表，另立方案。
- 列表 tight↔loose 切换、项内再开段落。Enter 在非空项上仍是续项，不是松散列表。
- 修改 `para|# heading` 这类 1-NL 邻接的 Enter。与 2026-08-02 规范中 ATX 标题末「补两个换行、形成一个可编辑空段」一致，保持现状。

## 硬换行规则（沿用 8 月 19 日）

连续反斜杠按奇偶性：偶数个之后的换行是软换行；奇数个里最后一个是硬换行标记。空格形式消耗全部行尾空格且不少于 2 个。Enter 与 Backspace 必须共用 `hard_break_marker_ending_at` / `hard_break_boundary_after`。覆盖 LF、CRLF、三个反斜杠。

## 结构分隔

容器的「结构分隔」不是一律 `\n\n`：

| 上下文 | 结构分隔写入 | 硬换行上 Enter |
|---|---|---|
| 顶层段落 | `\n\n` | 用 `\n\n` 替换硬换行边界（已有） |
| 列表项 | `\n{indent}{continuation_marker}` | 用该串替换硬换行边界 |
| 引用行 | `\n{continuation_prefix}` | 用该串替换硬换行边界 |
| ATX 标题末 | `\n\n` | 标题通常没有硬换行；不新增分支 |
| 表格单元格 | 跳到下一格；末行见下 | 不把硬换行写入单元格 |

`|` 仍表示光标，不属于源码。

### 列表

```
- first\
  second
```

光标在 `first` 后 Enter → `- first\n- |second`（反斜杠与续行缩进消失，后半成为下一项）。

光标在 `second` 前 Backspace → `- first|second`。

```
- item
para
```

这是同一项的懒延续，视觉为 `item para`。光标在 `item` 与 `para` 之间的换行上 Enter → `- item\n- |para`。不得得到 `- item\n- \npara`。

下一行已经是列表项时（`- a|\n- b`），Enter 仍插入新项，得到 `- a\n- |\n- b`，不要改写成 `- a\n- - b`。

### 引用

```
> first\
> second
```

光标在 `first` 后 Enter → `> first\n> |second`。

光标在 `second` 前 Backspace → `> first|second`。

```
> first
second
```

懒延续。光标在换行上 Enter → `> first\n> |second`。

两行都已有 `>` 时，Enter 插入空引用行：`> first\n> |\n> second`，不要写成 `> first\n> > second`。

硬换行合并不得跨越**新的同级列表项**：`- first\` 下一行是 `- second` 时，Backspace 不走硬换行合并，以免把两项粘成一项。

### 拆段空格

仅当光标紧邻恰好一个 ASCII 空格、且该空格不是硬换行标记的一部分时，拆段替换掉这一个空格：

- `left |right` 与 `left| right` 的段落 Enter 都得到 `left\n\n|right`
- `# left |right` 的标题中部 Enter 得到 `# left\n|right`

中词拆分 `lef|t` 不删任何空格。连续两个以上空格若构成硬换行，仍走硬换行分支。

### 表格末行

`EnterContext::TableCell` 增加末行信息（见实施计划）。

- 非末行：保持现状，光标移到下一行同列内容起点。
- 末行且当前行至少一格有非空白内容：在表末插入一行 `|  | … |`，列数与当前行相同，光标落在新行第一格内容处。
- 末行且当前行所有格子均为空白、且不是表头行：删除该空行，在表后建立块边界（与空列表项退出同类）。表头行即使「空」也只新增表体行，不删表头。

## 退格优先级

在现有链前面加入硬换行视觉行合并，且必须在 `get_marker_delete_range` 之前：

1. 空源码行
2. ATX marker 起点护栏
3. **硬换行下一行合并**（新）
4. 段首跨块边界
5. 块间单 grapheme 段
6. marker 删除

## 架构

生产改动限于 `crates/markdown/src/augmenter.rs`。不改 UI、app 生产逻辑、投影、hit-test。允许补 markdown 单测；若 app 层跨层测试期望被表格末行行为带动，只改期望不改生产路径。

复用已有 `hard_break_*`、`preferred_newline_sequence`、`emit_marker_break`。新增：

- `emit_marker_break_replacing(source, replaced, indent, marker)` — 与 `emit_block_break_replacing` 对称
- `backspace_join_hard_break_line` — 删 `marker.start .. current_byte`
- 列表/引用 Enter：先硬换行，再「换行上替换」，最后才是在非换行处插入

不引入新的 `AugmentKind`。

## 验证

`cargo test -p textora-markdown --lib augmenter::`

必须覆盖：列表/引用硬换行 Enter 与 Backspace、奇偶反斜杠、双空格、CRLF、懒延续 Enter、拆段空格、表格末行新增与空行退出。顶层段落硬换行既有测试不得失败。
