# 2026-06-27 上午 MdEditor Grapheme 提交审查报告

审查范围：2026-06-27 00:00 至 12:00 本地时间的 9 个提交，区间为 `d82d0a6f..9fcbc81c`。

## 提交范围

- `d3ea7956` 2026-06-27 07:54:51 +0800 test: add markdown visual grapheme map
- `a505908b` 2026-06-27 07:58:21 +0800 refactor: use grapheme source maps in markdown layout
- `c6f69c4a` 2026-06-27 08:02:36 +0800 fix: use grapheme hit testing in markdown editor
- `6537e7ba` 2026-06-27 08:04:37 +0800 fix: move markdown editor cursor by grapheme
- `e2dc0090` 2026-06-27 08:10:51 +0800 fix: make markdown selection grapheme-aware
- `bc353273` 2026-06-27 08:12:38 +0800 fix: sync markdown editor cursor after snapping
- `5186ee68` 2026-06-27 08:15:37 +0800 chore: clean up old char naming and fix clippy warnings
- `4c5134f1` 2026-06-27 08:15:41 +0800 chore: remove unintended test patch file
- `9fcbc81c` 2026-06-27 08:23:59 +0800 fix: correct grapheme_index_at_byte mid-cluster behavior and source map slicing

## 结论

WYSIWYG 编辑态的核心 cursor/hit-test/visual move 已经基本按 grapheme 语义落地，第 9 个提交也修掉了 `grapheme_index_at_byte` 中间落点和 wrapped segment source map slicing 的关键问题。

但 Preview-only selection/copy/highlight 还没有彻底落地。当前代码仍同时存在 tuple 协议、char-index 内部算法和 source-byte/line-byte 语义混用。也就是说，“不 split grapheme”在部分单元测试场景成立，但协议和数据模型还没有完全把 char 坐标赶出去。

## Findings

### P1 Preview 复制路径会把绝对 source byte 当作 line.text byte 切片

位置：

- `crates/markdown/src/layout/types.rs:16`
- `crates/markdown/src/layout/types.rs:678`
- `crates/markdown/src/edit.rs:177`
- `crates/markdown/src/edit.rs:230`
- `crates/markdown/src/selection.rs:192`
- `crates/markdown/src/selection.rs:272`

`FlatLineSourceMap` 和 `LaidOutLine::source_bytes_by_visual_grapheme` 的注释明确说 value 是“absolute source byte”。`materialize_line()` 也从 `span.source_range` 推导并写入源码绝对 byte。可是 `SelectionState::selected_text()` 通过 `byte_at_grapheme_on_line()` 取到这些值之后，直接执行 `&text[byte_start..byte_end]`。

这在第一行、无样式、source byte 恰好等于 line-local byte 的场景看起来正常；但在非首行样式文本、heading/list marker、WYSIWYG expanded span 等场景中，`byte_start`/`byte_end` 可能大于当前 `line.text.len()`，轻则复制错内容，重则运行时 panic。

建议修复方向：Preview copy 应该只使用 line-local grapheme to byte map；source byte map 只能用于 WYSIWYG 源码定位。需要拆出两个不同类型，避免一个字段同时承担“文本切片”和“源码定位”两种语义。

### P1 双击选词仍把 grapheme_pos 当 char index 使用

位置：

- `crates/markdown/src/selection.rs:99`
- `crates/markdown/src/selection.rs:111`
- `crates/markdown/src/selection.rs:140`
- `crates/markdown/src/selection.rs:167`

`ViewPos::grapheme_pos` 已经改名为 grapheme 坐标，但 `word_at_pos()` 内部仍构造 `Vec<char>`，并把 `pos.grapheme_pos` 直接当 `pos_char`。对 `e\u{0301}`、ZWJ emoji、variation selector 来说，grapheme index 和 char index 不相等。

结果是双击选词返回的 `ViewPos` 可能重新变成 char index，并被后续 selection/copy/highlight 当 grapheme index 使用。这个问题会让 preview 双击选择 emoji、组合音标文字时范围扩大或错位。

建议修复方向：word boundary 可以继续按 char class 判定，但输入输出必须通过 grapheme boundary 转换。更稳的是在 `FlatLine` 上建立 `line-local grapheme -> byte` 和 `byte -> grapheme`，word 算法内部用 byte range，边界输出再转回 grapheme。

### P2 搜索高亮仍以 char match index 调用 grapheme_x

位置：

- `crates/markdown/src/search.rs:57`
- `crates/markdown/src/search.rs:65`
- `crates/markdown/src/search.rs:79`
- `crates/markdown/src/search.rs:80`

`SearchState::update_if_needed()` 以 `Vec<char>` 查找 query，得到的是 char index；但绘制矩形时直接把 `start_ch_idx` 和 `end_ch_idx` 传给 `grapheme_x()`。当 query 或目标文本包含组合字符或 ZWJ emoji 时，搜索匹配范围和高亮矩形不再同一套坐标。

建议修复方向：搜索命中应记录 line-local byte range 或 grapheme range。若搜索逻辑保留 char scan，也必须在绘制前把 char boundary 转成 grapheme boundary，不能把 char index 当 grapheme index 传递。

### P3 Preview-only 协议仍是 tuple，类型层没有阻止 char 坐标回流

位置：

- `crates/ui/src/plugin.rs:34`
- `crates/ui/src/plugin.rs:35`
- `crates/ui/src/plugin.rs:37`
- `crates/ui/src/plugin.rs:138`
- `crates/ui/src/plugin.rs:140`

当前协议只把注释从 `char_pos` 改成了 `grapheme_pos`，但 `SetSelCursor(Option<(usize, usize)>)`、`WordAtPos(usize, usize)`、`LineRangeAtPos(usize, usize)` 仍然是裸 tuple。调用方无法从类型上区分第二个 `usize` 是 char、grapheme、byte，导致旧代码可以无声混入。

这不是单点崩溃 bug，但它解释了为什么 Preview selection 的状态仍然“迷”：协议名义上改成 grapheme，实际仍靠约定维持。

建议修复方向：新增 `PluginTextPosition { flat_line_idx, grapheme_index }` 和 `PluginTextRange` 等纯数据类型，再逐步替换 tuple。若需要兼容旧插件，应把兼容层集中在 `ui::plugin` 边界，而不是让 app/markdown 内部继续传 tuple。

## 已覆盖的中间态问题

第 9 个提交 `9fcbc81c` 修复了一个关键中间态问题：`grapheme_index_at_byte()` 原先对落在 grapheme cluster 内部的 byte 可能返回下一个 grapheme index，进而让 wrapped segment slicing 在 combining/ZWJ 附近切错。当前 `crates/markdown/src/grapheme_map.rs:59` 已改为先计算 cluster end，再判断 target 是否落在当前 cluster 内；`crates/markdown/src/layout/block.rs:493` 和 `crates/markdown/src/layout/block.rs:953` 也改为按 grapheme index slicing。

这个点不再列为当前 HEAD finding。

## 正向覆盖

- 新增 `VisualGraphemeMap`，统一了 visual grapheme 到 source byte 的基础转换。
- WYSIWYG hit-test 已从 visual grapheme 反查 source byte，避免点击和左右移动直接落入组合字符内部。
- `dispatch_wysiwyg_navigation()` 在 DocumentView snap 后回写 `SetCursorByte(snapped_byte)`，避免 plugin 内部 cursor 和真实 gap buffer cursor 分叉。
- WYSIWYG 编辑态 render 使用 full layout，cursor screen pos、visual move、select all 不再依赖仅可见行。

## 验证

已执行：

```text
cargo test -p edit-plus-markdown --lib -- grapheme
cargo test -p edit-plus-markdown --lib -- selection
cargo test -p edit-plus-app --lib -- wysiwyg
```

结果：

- `edit-plus-markdown -- grapheme`：25 passed
- `edit-plus-markdown -- selection`：4 passed
- `edit-plus-app -- wysiwyg`：8 passed

未执行 `./scripts/verify.sh`。本次是审查报告生成，没有修改生产代码；若按报告修复 P1/P2，完成后应执行完整验证。

## 审查环境备注

审查时工作区存在未跟踪文件 `test_cursor_pos.patch`。它不在这 9 个提交里，也未纳入本报告的行为判断。

