# MD WYSIWYG 换行与块元素关系审查报告

日期:2026-08-30
范围:`crates/markdown` 编辑侧(`augmenter.rs` / `view.rs` / `edit_context.rs`)与布局侧(`layout/*` / `source_line_map.rs` / `projection.rs`)
主题:物理换行 vs markdown 换行的处理,以及块级元素之间关系的维护

---

## 一、核心机制(已核实)

### 1.1 编辑数据流

```
按键 → EditCommand → EditIntent(app/dispatch/editor.rs)
  → MarkdownEditorView::plan_edit(view.rs:2866)
      ├─ 零宽选区过滤(view.rs:2868)
      ├─ 有选区 → plan_selection_edit(2894):删选区得"虚拟源码",
      │     在删除点跑 augmenter::augment_edit,再经
      │     selection_augmentation_edit_plan(2824)把偏移映射回真实文档
      ├─ DeleteForward → augmenter::augment_delete_forward(2875)
      └─ 其余 → engine.augment_edit → augmenter::augment_edit(augmenter.rs:41)
  → augmentation_edit_plan(view.rs:2802):空替换+光标不动 → Consume;
    空替换+光标移动 → MoveCursor;否则 → 单条原子 EditTransaction::replace
  → 应用到源码 → pulldown-cmark 全量重解析 → 重建块树/投影/布局
```

块结构不落盘、不增量维护:每次编辑是纯文本替换 + 全量重解析,所有"块感知"集中在 `classify_enter_context`(augmenter.rs:1627)。

### 1.2 物理行 ↔ 逻辑块映射

`SourceLineMap`(layout/source_line_map.rs)把源码空行 run 分三类:

- **块内空行**(代码块/metadata 内,自带渲染投影 `owns_rendered_line`)→ 按普通渲染行处理;
- **块间 run 首行** → `HiddenBlockSeparator`:不渲染、不可停留,高度固定为 `paragraph_spacing`,字节折叠进相邻行锚点;
- **run 第 2..N 行及文档末尾空行** → `EditableEmpty`:整行高、可点可输入的空段投影(`ProjectionOwnerId::EmptyLine`)。

块间垂直间距**纯样式驱动**,与源码有无空行无关;源码空行 ≥2 时才经 `reserve_extra_blank_source_lines`(types.rs:1367)追加高度。光标/点击在"虚拟间距"区靠 collapsed range(types.rs:396)和间隙中点吸附(view.rs:1721)归属。

### 1.3 Enter 决策矩阵(逐场景核实)

| 场景 | 行为 | 实现 |
|---|---|---|
| 段落内/末尾 | 拆段 `\n\n`,行内元素跨拆分点自动闭合/重开 | paragraph_enter_augmentation (1031), preserve_inline_elements_at_split (1338) |
| ATX 标题中间 | 单 `\n` 截断:前半保留标题,后半立即成段落 | heading_enter_augmentation (1053) |
| ATX 标题末尾 | `\n\n` 新空段(容器内 `\n`+前缀) | 同上 |
| Setext 标题任意处 | 统一在下划线后拆块,标题完整保留 | setext_heading_enter_augmentation (1096) |
| 列表项(含中间) | `\n{缩进}{续marker}` 拆两项;有序编号 +1;task 续项强制 `[ ]` | list_item_enter_augmentation (1125) |
| 空列表项 | 整行替换为缩进,退出列表 | emit_replace_current_line (923) |
| 引用行/空引用行 | 续 `> ` / 前缀截到最后一个 `>` 前,逐层退出嵌套 | blockquote_enter_augmentation (1180) |
| 表格单元格 | 跳下一行同列 / 末行加新行 / 空行退出表格 | 2013/2040 |
| 围栏代码块内 | 裸 `\n`(**含围栏行,见 P2**) | 1026 |
| 缩进代码块 | `\n` + 继承最近非空行缩进 | 1109 |
| Shift+Enter | 列表/引用内插 `\` 硬换行;标题/表格插 `<br>`;代码块裸 `\n` | 55–94 |

Backspace 方向的边界护栏链是完整的:`backspace_at_atx_heading_marker_start`(332)、`merge_into_preceding_block`(726,懒延续显式化)、`guard_unmergeable_leaf_boundary`(752)、`line_starts_new_sibling_block`(855)。

### 1.4 历史修复复核

M1(带选区回车)、M2(不可合并叶块守卫)、M3(Setext 分流)、M4(懒延续显式化)、L1(零宽选区)、L6(task 续项)、L12(ATX marker 拦截)、H1(代码块空行卡死)——均在码确认。CRLF 不变量(`newline_sequence_width_at/before` 406–420、`preferred_newline_sequence` 467)落实系统。

---

## 二、问题清单

### 中——正确性/结构风险

**P1. Delete(前向删除)完全没有块边界护栏,与 Backspace 严重不对称**
- 位置:augmenter.rs:131–137
- `augment_delete_forward` 只处理 `<br>` 移除和行内元素重开合并,其余回落默认逐 grapheme 删除。
- 触发:`文字|\n\n---` 段末 Delete → 删一个 `\n` → `文字\n---` → **段落变成 Setext H2**;`a|\n\nb` 段末 Delete → 两段静默合并。
- 修复方向:Delete 在段末/块边界对称地走边界分类(复用 `line_starts_new_sibling_block` 等)。

**P2. 代码块闭合围栏行回车可吞噬后续文档**
- 位置:augmenter.rs:1026 + classify_enter_context 1772–1786
- 闭合 ` ``` ` 行中间回车 → 围栏失效 → 代码块延伸至文末。Backspace 侧有 `line_starts_new_sibling_block` 防护,Enter 侧没有。
- 修复方向:分类器对"光标在围栏行"单列上下文;开头围栏行回车 → 跳到行尾进入代码块;闭合围栏行回车 → 在围栏行后 `\n\n` 退出代码块。

**P3. `edit_context.rs`(471 行)是无生产调用方的死代码,且与 H1 修复直接矛盾**
- 位置:crates/markdown/src/edit_context.rs(全文)
- 已 grep 全仓确认 `classify_markdown_edit_context` 仅被自身测试调用。其空行再推导(56–76)会把代码块内空行改判为 `HiddenBlockSeparator`——若被接线,H1 原样复活。与 augmenter 分类器重复实现,是漂移温床。
- 修复方向:删除或收编为 augmenter 的测试工具。

**P4. 隐藏分隔行高度硬编码 = paragraph_spacing,与真实样式间距不符**
- 位置:source_line_map.rs:262–268 + types.rs:1413–1417
- 标题(trailing 10.8px)或列表组(12px)后跟 ≥2 个空行时,可编辑空行与下一块重叠约 7px,光标画在文字上。
- 修复方向:`attach_layout` 消费真实块间 gap,而非按 paragraph_spacing 假设。

**P5. loose list 项内多段落零间距,且嵌套块间空行无高度补偿**
- 位置:block.rs:449–457 + types.rs:1399(只遍历顶层块)
- 触发:`- a\n\n\n  b` → 可编辑空行直接覆盖子段落文字。

**P6. 每次按键全量 reparse + 全文估计布局**
- 位置:view.rs:2994–2996;含代码块的子树一律放弃复用(types.rs:1316)
- 长文档每键 O(doc)。属架构性优化,**本轮不修**,另立项。

**P7. 光标移出屏外块后 active marker 永久残留**
- 位置:view.rs:780(只失效渲染窗口内的块)+ types.rs:1922(ensure_precise_range 跳过 precise 块)
- 触发:光标在标题内 → 滚出缓冲区 → 键盘移光标 → 滚回来标题仍显示 `# `。

### 低——边界与脆弱点

**P8. 空行几何三套实现并存,间距恢复表四处复制**
- source_line_map.rs:147 / types.rs:1367 / view.rs:1500–1531+1612–1642(回退路径),公式不同;间距恢复表在 block.rs:101–577、types.rs:2044–2079、types.rs:1632–1666、types.rs:1768–1803、render.rs:1287 重复。本轮只收敛 P4 涉及的不一致,全面重构另立项。

**P9. 列表 empty 误判与嵌套退出残留**
- augmenter.rs:1636–1640:`mark_item_content_seen` 不含 `Start(Tag::Item)`,只含子列表的父 item 被误判 `empty` → 回车把子列表提升为顶层。
- augmenter.rs:1131:嵌套空 item 退出残留 `"  "` 空白缩进行,再按 Enter 意外创建新同级 item。

**P10. marker 内部/多空格标题的边界**
- augmenter.rs:1691:光标在 `-` 后(byte < marker_end)不命中 ListItem → 裸 `\n` → 懒延续残留。
- augmenter.rs:1597:`hash_prefix = level + 1` 假定 marker 后恰好一个空格,`#  Title` 回车后半段带前导空格。

**P11. 单块重排只取首个 LaidOutBlock**
- types.rs:1685、1824、2094:`ctx.output.into_iter().next()` 静默丢弃后续块;`LazyLayout::new` 的 `assert_eq!`(types.rs:1217)此时会 panic。当前未触发,属埋雷。

**P12. 文档/死代码漂移**
- `docs/specs/2026-08-02-markdown-wysiwyg-enter-backspace-behavior.md` 仍称 Setext「不在范围内」(实现已有专门分支);未收录 `augment_insert_text` 空行修剪行为。
- `SourceLineMap` 的 `Heading/ListItem/BlockQuote/CodeBlock/TableCell` role 从未赋值,`is_hidden_block_separator` 等 API 仅测试使用。

### 已排除(本次复核确认不成立)

- `edit.rs:150` `ilog10(0)` panic:不可达,builder.rs:502,692 用 `ORDERED_LIST_PREVIEW_START = 1` 重置有序列表编号。
- `commands.rs:200–202` `plan_code_block` CRLF 偏移:仍存疑,属工具命令非 Enter 路径,建议后续补 CRLF 测试。

---

## 三、修复计划与状态

| # | 问题 | 阶段 | 状态 |
|---|------|------|------|
| P1 | Delete 边界护栏 | Stage A | 已修(`delete_forward_block_boundary`,augmenter.rs:242) |
| P2 | 围栏行回车护栏 | Stage A | 已修(`EnterContext::CodeBlockFenceLine` / `fence_line_enter_augmentation`) |
| P9 | 列表 empty 误判/嵌套退出残留 | Stage A | 已修(`mark_item_content_seen` 覆盖子 item;空白行逐层退出) |
| P10 | marker 内部回车/多空格标题 | Stage A | 已修(命中条件放宽;`classify_heading_hit` 扫描实际空白) |
| P4 | 隐藏分隔高度用真实间距 | Stage B | 已修(`attach_layout` 消费真实块间 gap;三处公式收敛一致) |
| P5 | 松列表项内间距/嵌套空行补偿 | Stage B | 已修(`reserve_nested_blank_source_lines`,layout_block 内递归补偿) |
| P7 | 屏外 active marker 残留 | Stage B | 已修(光标路径改用无窗口限制的失效) |
| P11 | 单块重排丢弃多余布局块 | Stage B | 已修(`relayout_multi_output_group`;断言改优雅降级) |
| P3 | 删除 edit_context.rs 死代码 | Stage C | 已修 |
| P12 | spec 更新/SourceLineMap 死 role 清理 | Stage C | 已修 |
| P6 | 每键全量 reparse 性能 | — | 不在本轮,另立项 |
| P8 | 空行几何全面收敛 | — | 本轮只修 P4 相关不一致 |
