# WYSIWYG Enter 键行为修复方案

> 目标：修掉两个可复现 bug — (1) 段末回车后光标处出现"很大"的空行，输入后又缩回；(2) 段间空行处回车只加空隙，不生成新段。

## 一、Bug 复现与根因

### Bug 1：段末回车出现异常大空行，输入后缩回

**复现**：源码 `hello world`（11 字节），光标在 11（段末），按 Enter。

**执行路径**：
- `dispatch_wysiwyg_augmented_enter` → `augment_edit`。
- `classify_enter_context` 命中 `TopLevelParagraphEnd`。
- `blank_line_augmentation` 返回插入 `"\n\n"`，`cursor_byte_after = 13`。
- 源码变成 `"hello world\n\n"`（13 字节），光标定位 13。

**光标绘制**：
- 位置 13 落在"空源码行"（`source_line_at_byte` 返回 `index=2, start=13, end=13`）。
- 走 `empty_source_line_cursor_screen_pos` → `empty_source_line_metrics`。
- 关键一行（`crates/markdown/src/view.rs:900`）：
  ```rust
  let line_gap = count_newlines_between(source, previous_byte, source_line.start).max(1) as f32;
  cursor_y = previous_flat_line.rect.y + previous_flat_line.rect.h * line_gap;
  //           = prev.rect.y + line_height * 2
  ```
- 结果：光标 y 位于 **段落顶部 + 2 × line_height**。

**输入 `x` 后源码变 `"hello world\n\nx"`**，第二段真正生成，`layout_block` 里段间隔仅 `paragraph_spacing ≈ 0.5 × line_height`（见 `block.rs:71`）。第二段位置 = 段一底 + `paragraph_spacing` = **段落顶部 + 1.5 × line_height**。

**根因**：empty_source_line 计算把每个换行按 `line_height` 折算；layout 里 CommonMark 的 `\n\n`（块分隔）折算成 `paragraph_spacing`（仅 0.5×line_height）。两套公式不自洽，光标先偏低 0.5×line_height，输入后缩回。

### Bug 2：段间空行回车"只加空隙"

**复现**：源码 `"para1\n\npara2"`（12 字节）。光标在 6（两个 `\n` 之间的空行）。按 Enter。

**执行路径**：
- `classify_enter_context(source, 6)`：解析器走到 `End(Paragraph)` 事件时，`current_byte=6` 落在段一 `range` 内，但 `content_end_without_trailing_newline` = 5，`current_byte != end` → 判定 `ParagraphInterior`。
- `augment_edit` 对 `ParagraphInterior` 返回 `None`。
- 回落到 `EditCommand::InsertNewline`，在字节 6 插入单个 `\n`。
- 源码变 `"para1\n\n\npara2"`，光标 7。

**视觉结果**：`\n\n\n` 相当于 2 个块分隔换行 + 1 个多余空行。`reserve_extra_blank_source_lines` 有个门（`types.rs:827`）：当 `cursor_byte` 落在换行 run 内（`newline_run_start < cursor < current_start`）就 **skip 保留**。第一次 Enter 后光标就在换行 run 里，第二段不下移，视觉上仅仅"多了个空隙"，不像"新段落"。

**再按一次 Enter**：光标又插入 `\n` → `"para1\n\n\n\npara2"`，run 变长，直到光标出 run 或第二段被推下去，用户才感觉"新段真的出来了"。

**根因**：
1. 分类器缺少"空的块间空白行"这个类别，落到 `ParagraphInterior`（错误）或 `Other`（同样返回 None）。
2. `augment_edit` 在这一情形没有针对性行为，退回到纯 `InsertNewline`，语义不对。
3. `reserve_extra_blank_source_lines` 的"光标在 run 里就 skip"策略让插入的空行在光标未离开时不占空间，加剧错觉。

## 二、修改方案

### 方案 A（推荐）：把"空源码行"垂直度量与 layout 对齐 + 增补分类

包含两块改动。

#### A1. 修正 `empty_source_line_metrics` 的垂直换算

文件：`crates/markdown/src/view.rs`，函数 `empty_source_line_metrics`（约 868–925 行）。

新公式：把 `\n` 计数拆成"块分隔部分"和"额外空行部分"，与 `layout_block` 一致。

```text
gap(newlines N) =
    0                                              if N <= 1
    paragraph_spacing                              if N == 2
    paragraph_spacing + (N - 2) * line_height      if N >= 3
```

具体：
- 计算 `newline_count = count_newlines_between(source, previous_byte, source_line.start)`；
- 从 `MarkdownStyle` 拿 `paragraph_spacing` 与 `line_height`（新增引用；`empty_source_line_metrics` 目前不知道 style，需要在 `MarkdownEditorView` 或 `LazyLayout` 上暴露 `paragraph_spacing` 快照，或者在 `rebuild_layout` 时把 `paragraph_spacing` 缓存到 engine 的字段）；
- 返回 `y = previous_flat_line.rect.y + previous_flat_line.rect.h + gap(newline_count)`（注意：现在返回的是**空源码行的顶部**，不再是"prev top + line_gap × line_height"）；
- 对 `next_line` 分支对称处理：`y = next_flat_line.rect.y - gap(newline_count)`。

要点：
- **保持 hit_test 一致**：`empty_source_line_byte_at_doc_y`（`view.rs:973`）也走 `empty_source_line_metrics`，公式同步就自动一致，不需要额外改点击命中。
- **测试对齐**：`cursor_after_trailing_newline_moves_to_empty_paragraph_line` 之类的既有测试断言 `cursor_y > 24.0`（其实是"大于一整行"），新公式下若 `paragraph_spacing = 0.5*line_height`，`cursor_y = line_height + 0.5*line_height = 1.5*line_height ≈ 21`。**需要把这些测试断言改成 `cursor_y >= line_height + paragraph_spacing - eps`**，用真实模型量而非硬编码 24。列出需要复查的测试：
  - `cursor_after_trailing_newline_moves_to_empty_paragraph_line`
  - `cursor_after_trailing_newline_uses_physical_line_metrics`
  - `cursor_after_trailing_newline_roundtrips_to_empty_line_byte`
  - `extra_empty_source_line_reserves_vertical_space_before_next_paragraph`
  - `cursor_after_trailing_blank_line_uses_nearby_paragraph_position`

#### A2. 新增 `EmptyBlockSeparatorLine` 分类，Enter 行为对齐

文件：`crates/markdown/src/view.rs`，`EnterContext` 枚举 + `classify_enter_context` + `augment_edit`。

##### 分类器改动
在 `classify_enter_context` 主 pass 前（或 `Other` 兜底前）加一个显式判断：
```text
若 source_line_at_byte(current_byte) 是"空行"（start == end）
    且该行不属于列表/引用/代码块/表格 上下文
    → EnterContext::EmptyBlockSeparatorLine { has_prev_block, has_next_block }
```
其中 `has_prev_block` / `has_next_block` 通过扫描前后最近的非空行拿到。位置上：这个分支应放在段落分类**之前**，以便优先命中；`ParagraphInterior` 的判定需要额外确认"cursor 落在段一 range 内但已经越过了 content 尾"这种情况不会误吞。

##### `augment_edit` 分支
`EnterContext::EmptyBlockSeparatorLine` 的处理：
- **不再插入 `\n\n`**（会把两段变三段的空行数升级到 4 个换行，视觉更糟）；
- 语义："我在段间空行上按 Enter，就是想把这一空行推成新段的位置"；
- 具体：**保持源码不变**（`insert_text = Some(String::new())`），**替换范围也是空**，`cursor_byte_after` 设为下一个块的起点 `current_start`（若存在）或 `current_byte`。也就是把光标直接跳到下一段首行。

如果用户希望"Enter 就是要生一个新段"（更符合 Word / Typora 直觉），备选行为：
- 在当前空行处插入一个额外 `\n`，让空行数达到 3；
- 关键：**同时移除** `reserve_extra_blank_source_lines` 里"光标在 run 里就 skip"的门（见 A3），让新增的空行立即在视觉上体现。

两种选择建议先跟用户确认；下文默认取"直接跳到下一段首"（更少 surprise，也不改动源码）。

##### 段末 `TopLevelParagraphEnd` 微调
现在 `blank_line_augmentation` 插入 `"\n\n"`，光标跳 +2。经 A1 修正后，光标 y 会落在"下一段应在的位置"（prev_bottom + paragraph_spacing），视觉不再"很大"。保持插入 `\n\n` 的现有语义即可，不必改。

#### A3. `reserve_extra_blank_source_lines` 的光标 gate 复审

文件：`crates/markdown/src/layout/types.rs:793–837`。

现状：`edit_ctx.cursor_byte` 落在换行 run 里（`newline_run_start < cursor < current_start`）就 **skip** 保留额外空行高度。目的估计是避免光标位置在 run 内时布局跳动。

问题：这使得"输入前布局"和"输入后布局"总不一致；结合 A1 之前的空行公式，用户会看到 caret 和 following block 位置不吻合。

建议：**移除这个 skip 分支**，让 layout 忠实反映源码换行数；A1 保证 caret 位置也走同一套公式，二者始终吻合。可能需要观察：连续按 Enter 时，如果 layout 每次都下推，滚动条会跳；如果这引发新问题，再引入更精细的策略（例如仅在换行 run 长度 = 2 时不保留）。

### 方案 B（保守，只治 Bug 1）

若不想动分类和 gate 逻辑，仅做 A1 一步：把 empty_source_line 的垂直度量对齐到 `paragraph_spacing`。Bug 1 完全消失，Bug 2 部分缓解（"很大空行"变小，但"要按两次 Enter"仍存在）。

不推荐单独用 B，因为 Bug 2 的根源是分类不完整，不做 A2 就永远残留。

## 三、实施顺序（拆子任务）

1. **测试先行**：为下列场景各写一个失败测试
   - 段末回车后，caret y 应等于 `prev_bottom + paragraph_spacing`（Bug 1）；
   - `"para1\n\npara2"`，光标 6，Enter 后源码不变且光标为 7（跳到 para2 首）（Bug 2）；
   - `"para1\n\n\npara2"`，光标 7（run 中间），layout 保留 1 行额外空行高度，无论光标位置（A3）。

2. **A1 实施**：修 `empty_source_line_metrics`；同步既有测试硬编码断言（改为按 `paragraph_spacing` 计算的表达式）。

3. **A2 实施**：新增 `EmptyBlockSeparatorLine` 分类；`augment_edit` 补分支；补测试覆盖列表 / 引用 / 代码块 / 表格上下文里的空行不被误命中。

4. **A3 实施**：移除 `reserve_extra_blank_source_lines` 中的光标 gate；跑全量回归观察连续 Enter 的视觉平滑度。

5. **验证**：`./scripts/verify.sh`，重点看 `crates/markdown/src/view.rs` tests + `crates/app/src/dispatch` tests。

## 四、需要与用户对齐的一个点

Bug 2 中，用户在 `"para1\n\npara2"` 的空白行按 Enter，期望是：
- (i) 光标直接跳到 `para2` 首行（不改源码）；还是
- (ii) 真的生成一个"更空的段"，光标停留在新的空行，等下一次输入？

方案 A2 默认取 (i)。若用户实际想要 (ii)，改 `augment_edit` 分支即可（配合 A3 让新增空行立即在布局中体现）。

---

## 五、同类问题（追加扫描发现）

这些跟 Bug 1 / Bug 2 是同一族根因，一起修比较划算。

### 5.1 段落中间按 Enter 只做软折行，语义不对（Bug 2 变体）

`classify_paragraph_hit`（`view.rs:1725`）：光标不在段末 → `ParagraphInterior`；`augment_edit` 返回 None → 落到 `InsertNewline` → 只插一个 `\n`。

问题：CommonMark 里段中单 `\n` 是软折行，仍属同一段。用户按 Enter 的直觉是"把当前段一分为二"，实际得到的是软折行，视觉上看不出变化（同一段继续），但源码里多了个 `\n`。再按一次 Enter 才能真的拆段。

修：`ParagraphInterior` 分支改为插入 `"\n\n"`，`cursor_byte_after = current_byte + 2`，跟 `TopLevelParagraphEnd` 对称。

### 5.2 Blockquote / List 内段落中间按 Enter 只插单 `\n`

blockquote：`EnterContext::BlockQuoteLine { empty:false, at_end:false }` → `augment_edit` 返回 None → 单 `\n`。这不但没拆段，还会破坏 blockquote 结构（新行没有 `> ` 前缀，pulldown 视其为退出引用）。

List item 内 paragraph 中间：走 `End(TagEnd::Item)` 分支，只要 current_byte 落在 item 内就返回 `ListItem`。但如果是**多行的 item**，光标在 item 中间某行的中段，处理是"再开一个 marker"，可能不是用户想要的（用户可能只想在当前 item 内软折行；也可能想拆两条 item）。

修：
- BlockQuote 段中：插入 `"\n> "` 强制续引用，与 `at_end` 分支一致；
- ListItem 中间：短期保持"续 marker"，但**至少不能是普通 `\n`**，也要评估把 item 里的"段末"跟"行末"区分——现在 `ListItem { at_end }` 的 `at_end` 拿到了但没被用（`ListItem { bullet, empty, at_end: _ }`）。

### 5.3 Heading 中间按 Enter 直接从中截断

`classify_heading_hit`（`view.rs:1707`）：只要 `current_byte >= content_start` 就返回 `Heading { level, at_end }`；`augment_edit` 里 `EnterContext::Heading { .. }` 无视 `at_end` 一律 `blank_line_augmentation`。

问题：光标在 `# hello world` 中间某字符，按 Enter 会把标题从中间截断，插 `\n\n`，形成"半个标题 + 半个段落"。用户直觉应是"在标题内换段"或"标题分成两半"，但两种都不太对。

修：区分 `at_end`：
- `at_end == true`：现行 `\n\n`（新段）；
- `at_end == false`：可选方案——(a) 直接把光标跳到标题末尾再插 `\n\n`（更符合"我按 Enter 想离开标题写正文"）；(b) 保持 None → 单 `\n`（但 CommonMark 里 heading 是单行，源码里插 `\n` 就把标题截断成"半标题 + 段落"，会跳格式）。推荐 (a)。

### 5.4 HR / 未分类块之后按 Enter 走 `Other`

分类器对 `ThematicBreak`、`Definition`、`HtmlBlock`、`FootnoteDefinition` 等都落到 `Other` → None → 单 `\n`。在这些块之后按 Enter，用户期望"新段"，实际得到"上一块尾多个 `\n`"（可能被 pulldown 视为空源码行或语义不清）。

修：`Other` 兜底改成"看看当前源码行是不是空的"——若是空行且属于块与块之间，走 `EmptyBlockSeparatorLine`（跟 5.5 合并）；否则再退回 None。

### 5.5 空源码行的分类归口（跟 Bug 2 A2 合并）

统一逻辑：
```
若 source_line_at_byte(cursor) 是空行
    → EmptyBlockSeparatorLine
```
覆盖：
- 两段之间空行（Bug 2 主场景）；
- 段末带 `\n` 光标停在 `\n` 后（`hello\n` 光标 6）；
- 文档开头 `\n` 之前光标 0（首行空）；
- 连续多个空行中的任一个；
- HR 之后紧跟的空行。

用同一 `augment_edit` 分支处理（默认：光标跳到下一非空块首，若不存在则跳到末尾）。

### 5.6 List / BlockQuote 空 item 清空后 caret 位置抖动（Bug 1 变体）

`ListItem { empty: true }` 分支把当前行 `start..line_end` 全部替换为 `""`；`cursor_byte_after = start`。此时源码里那一行变成空源码行，caret 走 `empty_source_line_cursor_screen_pos`。

问题：清空前一 flat_line 是列表 item，间距 = `list_item_spacing`（甚至 `list_group_spacing`），跟 `empty_source_line_metrics` 目前按 `line_height * N` 算不匹配。修 A1 后会好，但要**测试覆盖** list 和 bq 的这类跳变，别只在 paragraph 之间验。

### 5.7 `reserve_extra_blank_source_lines` 的光标 gate 与 caret 不同步（Bug 1 根因之一）

参见 A3。此 gate 影响所有"光标停在换行 run 中"的场景（包括本节 5.1–5.5 修完之后的情形，因为多个 `\n` 相邻的中间点很常见）。移除后要观察连续 Enter 时的滚动/闪烁行为。

### 5.8 `dispatch_wysiwyg_augmented_enter` 里 replace_range 空到空的副作用

`dispatch/wysiwyg.rs:184`：只要 `replace_range` 是 `Some(range)` 就把 selection_anchor 设为 `range.start`。TableCell 分支目前传 `Some(current_byte..current_byte)`，等效于"设一个零宽 selection"。当 insert_text 也是空、只做 cursor 跳转时，这段 anchor 设置纯粹是死代码（下面又会被 `cursor_move_to_offset(augmented.cursor_byte_after)` 覆盖），无副作用但混淆。

修：`replace_range == None || range 非空` 才处理 selection anchor；把 TableCell 的 `Some(current_byte..current_byte)` 改成 `None`。

### 5.9 Backspace 的 augment 是完全空的（对称缺口）

`AugmentKind::Backspace => None`，且 `dispatch_wysiwyg_augmented_backspace` 直接透传给普通 Backspace。这不是本次 Enter bug 的一部分，但同类：例如"列表 marker 头 backspace 应变段落"、"引用 `> ` 头 backspace 应退出引用"这些 markdown-aware 行为都缺失。建议单开一个方案单，不放进本次 patch。

## 六、修订版实施顺序

考虑到 5.1 / 5.2 / 5.3 是**行为**变化，需要跟用户确认再动。原计划的 A1 / A3 只是修 caret 位置和布局一致性，改动风险小，可以先做。建议顺序：

1. **测试红灯**：Bug 1 / Bug 2 主场景 + 5.6 / 5.5 覆盖。
2. **A1 + A3**（caret / layout 对齐）— 无行为变化，只是视觉不再跳。
3. **A2 + 5.5**（`EmptyBlockSeparatorLine` 统一分类）— 单一新行为，先跟用户确认默认取向 (i) 还是 (ii)。
4. **5.1 段中拆段**：确认 Word 语义后再改。
5. **5.2 / 5.3 blockquote / heading 中间**：跟 5.1 一批走。
6. **5.4 HR / Other 兜底**：合并到 5.5 一次做。
7. **5.8 dispatch 清理**：低风险，最后跟着做。
8. **5.9 Backspace**：另开方案单。

