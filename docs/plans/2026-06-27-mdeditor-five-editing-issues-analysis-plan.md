# Mdeditor Five Editing Issues Analysis And Fix Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:systematic-debugging` first, then `superpowers:test-driven-development`, then `superpowers:subagent-driven-development` or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 Markdown WYSIWYG 编辑视图中的 IME preedit 布局、光标绘制/点击、列表/编号编辑语义、以及 fenced code block 输入定位问题。

**Architecture:** 保持现有分层：`app` 层负责窗口事件、IME composition 状态、`DocumentView` 写入和 plugin 同步；`markdown` plugin 负责 Markdown source 到视觉 layout/source map/cursor rect/hit-test 的投影；`ui` 只承载纯数据协议。核心方向是把“编辑态临时文本”和“Markdown block marker”纳入同一条可测试的 WYSIWYG 布局链路，而不是在 app 层用额外 overlay 或启发式点击补丁修症状。

**Tech Stack:** Rust workspace；`edit-plus-app` 的 `app_lifecycle` / `app_renderer` / `app_window` / `dispatch`；`edit-plus-markdown` 的 `view` / `edit` / `layout` / `builder`；`edit-plus-ui` 的 plugin 纯数据协议；验证使用 `cargo test`、`cargo check`、`cargo fmt`，重大修改后执行 `./scripts/verify.sh`。

## Global Constraints

- 全程遵守 `AGENTS.md`：中文沟通，行动前说清方案；遇 bug 先写复现测试再修。
- 单阶段修改超过 3 个文件必须拆分为子任务；每次提交前必须确保编译通过。
- `crates/ui` 只能定义纯数据协议，绝对禁止依赖或访问 `crates/app` 状态结构体。
- `crates/app` 负责从 `DocumentView` / IME event / plugin response 映射数据；`crates/markdown` 不得直接依赖 app 状态。
- WYSIWYG 光标、hit-test、IME candidate rect、preedit 绘制、编辑命令必须以同一份 Markdown flat line/source map 为事实来源。
- Markdown marker 进入编辑态时不得无意义改变后续 block 的 `y`；只有真实源码内容变化导致换行变化时，后续 block 才能移动。
- 用户可见光标位置必须落在 UTF-8 char boundary 和 grapheme boundary。
- 命名必须精准自解释，禁止新增 `data` / `info` / `temp` / `flag` 等宽泛名称。

---

## Scope Check

这 5 个问题横跨 IME、layout、hit-test、编辑命令、code block source map。它们不是一个单点 bug，但都落在 Markdown WYSIWYG 编辑态的“source byte <-> visual grapheme <-> pixel rect”链路上。建议分 5 个阶段实现，每阶段都有独立失败测试和可验证交付，避免继续叠加防御性补丁。

不纳入本计划：

- 普通文本编辑器 IME 行为。
- Markdown table 的结构化编辑。
- Preview-only selection/copy/search 的 grapheme 迁移遗留问题。
- 重写 Markdown parser。

---

## Current Evidence

### IME / preedit

- `crates/app/src/app_renderer.rs` 已在 WYSIWYG plugin 自渲染分支追加 `preedit_text_vertices()`，并用 `wysiwyg_cursor_window_rect()` 定位 preedit overlay。
- `crates/app/src/app_window.rs` 已用 `wysiwyg_cursor_window_rect()` 给 OS 设置 IME candidate area，并把 `preedit_advance_px` 加到候选窗 x 上。
- 但 `crates/markdown/src/edit.rs::EditContext` 仍只有 `cursor_byte`。`crates/markdown` 布局不知道当前 preedit 文本，所以后面的内容不会被 transient text 推开，也不会重新 wrap。
- `preedit_advance_px` 当前按完整 preedit text 测量，未区分 IME 光标在 preedit 中间的情况；候选窗位置应使用 preedit cursor 之前的 advance，而不是完整 preedit 宽度。

### 光标绘制 / 点击

- `crates/markdown/src/view.rs::cursor_screen_pos()` 用 flat line 的 `rect.y` 和 `font_size.min(rect.h)` 计算 cursor rect，已经具备垂直居中的意图。
- 当前点击进入 WYSIWYG 走 `dispatch/mouse.rs -> HitTestByte -> SetCursorByte`。marker 展开或 preedit 参与布局后，第一次点击可能基于旧 layout 命中，第二帧才出现新 layout，容易导致点击位置和最终光标不一致。
- `dispatch/mouse.rs` 的 WYSIWYG 分支在 mouse down 时就设置 selection anchor；如果后续 release 清空空选区的逻辑没有覆盖“点击导致二阶段重排后 byte 改变但无真实拖拽”的情况，单击容易表现成选择状态。

### 列表 / 编号 active marker

- 当前已有 `crates/markdown/src/edit.rs::ActiveBlockMarker`，并在 heading/list/blockquote active 时调用 `prepend_marker_to_line()`。
- `prepend_marker_to_line()` 会直接改 `LaidOutLine.text`，并清空 `line.shaped` / `text_layout`。这会让 marker 行回退到启发式宽度，并把 marker 混入正文 hit-test。
- 对列表而言，`layout_block()` 中 active list 会从 source 拆行，首行去掉 marker 后再布局，随后又把 marker prepend 回 first line。这个“先去 marker、再 prepend marker”的模型容易让编号 marker 和正文 source map 边界重复/漂移。

### 空编号 / 空列表回车

- `crates/markdown/src/view.rs::augment_edit()` 对所有 `ListItem` 的 Enter 都返回 `"\n{next_marker}"`。
- 该逻辑没有判断当前 list item 是否为空，也没有区分 cursor 是否在 marker 后的空内容处。
- 因此 `1. ` 后不输入内容直接回车时，当前实现会继续插入 `2. `，而不是删除当前 marker 并退出到普通段落。

### Fenced code block

- `crates/markdown/src/layout/block.rs` 的 `CodeBlock` layout 为每个 code line 创建 `LaidOutLine`，但 `source_bytes_by_visual_grapheme: None`。
- `LazyLayout::build_flat_lines()` 遇到没有 explicit source map 的 flat line 会用 block line fallback。对 code block 来说，fallback 很难知道 fenced marker 后每一行 code content 的真实 source byte。
- 这解释了“代码区块编辑，输入的文字响应到了下面的段落里”：点击 code line 后无法稳定回投影到 fenced code 内部 source byte，可能落到 code block 结束 fence 或后续 paragraph 的 byte。

---

## Root Cause Model

当前系统有三类内容被当成“普通文本行”处理，但它们的语义不同：

```text
1. Markdown source text
   真实写入 DocumentView 的字节流

2. Visual rich text
   Markdown 折叠后的排版文本

3. Editing transient text
   active marker、inline marker、IME preedit 这类只在编辑态可见或临时存在的文本
```

现在 inline marker 已部分通过 `materialize_line()` 建 source map；block marker 用 `prepend_marker_to_line()` 混入 line text；IME preedit 则仍在 app 层 overlay。三者没有同一个 layout/source-map 协议，所以会出现：

- preedit 可以画出来，但后文不会被推开；
- marker 可以显示，但光标/点击用的 shaped data 被清空；
- list item 的 marker 和正文 byte 边界混淆；
- code block 没有行内 source map，点击回投影到错误块。

目标模型应统一为：

```text
SourceByte
  <-> VisualGraphemePos
  <-> MarkdownEditorLineLayout
  <-> PluginDocumentRect
  <-> WindowRect
```

其中 `MarkdownEditorLineLayout` 明确区分三种 segment：

```rust
enum EditorVisualSegmentKind {
    SourceBacked,
    MarkdownMarker,
    ImePreedit,
}
```

跨层协议不一定暴露这个 enum；它可以先作为 `crates/markdown` 内部类型落地。关键是 source-backed segment 负责真实 byte 映射，marker segment 负责 marker source range，preedit segment 负责临时宽度和绘制，但不写入源码。

---

## Approaches

### Approach A: 继续在现有 line text 上拼接 marker/preedit

做法：把 preedit 也像 block marker 一样拼进 `LaidOutLine.text`，扩展 `prepend_marker_to_line()`。

优点：改动最小，能快速看到 preedit 推开后文。

缺点：会继续混淆 source-backed text、marker、preedit；source map 需要更多特殊 sentinel；点击、选择、wrap 边界会更难稳定。这个方向不推荐。

### Approach B: 引入内部 segment 化编辑行模型

做法：在 `crates/markdown` 内部新增编辑态 line materialization 结果，按 segment 表达 source-backed 正文、Markdown marker、IME preedit。layout 根据 segment 生成 text/layout/source map，hit-test 根据 segment kind 回到 source byte 或 cursor byte。

优点：根因层面统一 IME、marker、code line source map；能以测试驱动逐项替换，不需要推翻 plugin 协议。

缺点：需要动 `edit.rs`、`layout/block.rs`、`layout/types.rs`、`view.rs`，必须分阶段。

### Approach C: active block 直接退回源码全文编辑

做法：光标进入 heading/list/code block 时，整个 block 用原始 source text 渲染，类似源码编辑器。

优点：source map 最简单，code block 和空列表语义容易处理。

缺点：体验退化明显；大 block 进入编辑态会高度和 wrap 大幅跳变；与已有 Typora 式局部展开方向冲突。只适合作为 code block 的局部 fallback，不适合作为全局方案。

**Recommendation:** 采用 Approach B。短期允许 code block 先用 source-backed line map 做最小修复，但 marker/preedit 不再继续堆到无类型 line text 上。

---

## File Structure

- Modify: `crates/ui/src/plugin.rs`
  - 新增纯数据 IME preedit message/query 所需类型；不引入 app 类型。
- Modify: `crates/app/src/app_lifecycle.rs`
  - 在 IME `Preedit` / `Commit` / `Disabled` 时同步 WYSIWYG plugin preedit state。
- Modify: `crates/app/src/app_renderer.rs`
  - 保留 WYSIWYG preedit overlay 或迁移到 markdown vertices；第一阶段只负责 preedit advance cursor 位置正确。
- Modify: `crates/app/src/app_window.rs`
  - candidate area 使用 preedit cursor advance，不使用完整 preedit advance。
- Modify: `crates/app/src/dispatch/mouse.rs`
  - WYSIWYG 点击改为二阶段命中，避免 marker/preedit reflow 后单击变选区或落点漂移。
- Modify: `crates/app/src/dispatch/wysiwyg.rs`
  - smart Enter 支持空列表/空编号退出。
- Modify: `crates/markdown/src/edit.rs`
  - 扩展 `EditContext`，新增 preedit/segment materialization 数据结构与空 list item 判断纯函数。
- Modify: `crates/markdown/src/layout/block.rs`
  - marker/preedit/code line source map 的主要落地点。
- Modify: `crates/markdown/src/layout/types.rs`
  - flat line source map、active marker 高度稳定性、code block 回投影测试。
- Modify: `crates/markdown/src/view.rs`
  - plugin message/query、hit-test/cursor rect、augment edit 语义。

---

## Task 1: Preedit 成为 Markdown 编辑态布局输入

**Files:**

- Modify: `crates/ui/src/plugin.rs`
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/markdown/src/edit.rs`
- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/markdown/src/layout/block.rs`

**Interfaces:**

- Produces: `PluginMessage::SetPreedit { text: String, cursor: Option<(usize, usize)> }`
- Produces: `EditContext { cursor_byte, preedit: Option<PreeditContext> }`
- Produces: `PreeditContext { text: String, cursor_byte_in_preedit: usize }`
- Consumes: app IME `Preedit(text, cursor)` event and existing WYSIWYG `SetCursorByte`

- [ ] 写失败测试 `wysiwyg_preedit_reflows_text_after_cursor`：
  - source: `abcd efgh ijkl mnop`
  - narrow width 让插入 `"中文中文"` 后当前行发生 wrap。
  - 设置 cursor 在 `efgh` 前，再发送 preedit。
  - render 后断言 cursor line 的 flat line text 包含 preedit，且 preedit 后面的 source-backed text x 坐标右移或进入下一 visual line。
- [ ] 写失败测试 `wysiwyg_preedit_cursor_rect_uses_preedit_cursor_prefix`：
  - preedit text 为 `"nihao"`，preedit cursor 为 `(2, 2)`。
  - 断言 IME candidate x 使用 `"ni"` 的 advance，不是 `"nihao"` 的完整 advance。
- [ ] 在 `ui::plugin` 中新增纯数据 message；app 的 IME `Preedit` 分支在 WYSIWYG active 且搜索框未聚焦时发送给 plugin。
- [ ] 在 `PreviewEngine` 保存 preedit context，`set_edit_ctx()` 组合 cursor byte 与 preedit context。
- [ ] 在 `materialize_line()` 或新的 segment materializer 中，当 source byte 命中 cursor 所在行时插入 preedit segment。
- [ ] preedit segment 的 source map 使用 cursor byte 重复映射；hit-test 落在 preedit 内时返回 cursor byte，不允许产生不存在的 source byte。
- [ ] commit/disable 后发送 empty preedit，并触发 cursor block invalidate。

**Verification:**

- Run: `cargo test -p edit-plus-markdown --lib -- preedit`
- Run: `cargo test -p edit-plus-app --lib -- ime`

---

## Task 2: 光标 rect、点击和选择状态统一到二阶段命中

**Files:**

- Modify: `crates/app/src/dispatch/mouse.rs`
- Modify: `crates/markdown/src/view.rs`
- Test: `crates/app/src/app_tests.rs` 或 `crates/app/src/dispatch/mouse.rs`

**Interfaces:**

- Consumes: `PluginQuery::HitTestByte`
- Consumes: `PluginQuery::CursorScreenPos`
- Produces: helper `set_wysiwyg_cursor_from_point(px, py) -> usize`

- [ ] 写失败测试 `single_click_on_wysiwyg_after_marker_reflow_does_not_leave_selection`：
  - 构造 WYSIWYG markdown 文档 `1. hello`。
  - 单击正文，让 marker 进入 active 状态。
  - 断言 `selection_anchor == None`，cursor byte 为二阶段命中后的 snapped byte。
- [ ] 写失败测试 `cursor_rect_is_vertically_centered_in_flat_line`：
  - render 一个 heading、list item、paragraph。
  - 分别设置 cursor，断言 `cursor_y == fl.rect.y + (fl.rect.h - cursor_h) * 0.5 - scroll_y`。
  - 断言 cursor height 使用语义常量，例如 `ACTIVE_CARET_HEIGHT_RATIO` 或 `font_size.min(line_h)`，不使用散落魔法值。
- [ ] 在 WYSIWYG mouse down 中执行：
  - 第一次 `HitTestByte` 得到 candidate。
  - `DocumentView.cursor_move_to_offset(candidate)` 后回读 snapped byte。
  - 发送 `SetCursorByte(snapped)` 并要求 plugin 对 cursor block 同步刷新。
  - 使用同一个 mouse point 再次 `HitTestByte`，得到 final byte。
  - 写回 final snapped byte。
- [ ] mouse release 时只在真实拖拽距离超过阈值时保留 selection anchor；单击或二阶段 reflow 引起的 byte 微调不算拖拽。
- [ ] 把 cursor rect 高度常量集中到 markdown view 或 style 设置中，避免 app/markdown 两套高度算法。

**Verification:**

- Run: `cargo test -p edit-plus-app --lib -- wysiwyg`
- Run: `cargo test -p edit-plus-markdown --lib -- cursor_rect`
- Run: `cargo test -p edit-plus-app --lib -- mouse`

---

## Task 3: 列表/编号 marker 从正文 wrap 中拆出

**Files:**

- Modify: `crates/markdown/src/edit.rs`
- Modify: `crates/markdown/src/layout/block.rs`
- Modify: `crates/markdown/src/layout/types.rs`

**Interfaces:**

- Produces: `EditorVisualSegmentKind::MarkdownMarker`
- Produces: marker lane width from shaped marker text
- Consumes: existing `ActiveBlockMarker`

- [ ] 写失败测试 `ordered_list_active_marker_hit_test_roundtrips_each_digit`：
  - source: `12. abc`
  - cursor 进入 list item。
  - 对 `1`、`2`、`.`、空格、`a` 的 cursor rect 中点做 hit-test。
  - 断言不会卡在数字中间，返回 byte 是 marker/source 的合法边界。
- [ ] 写失败测试 `list_item_y_stable_when_marker_becomes_active`：
  - source: `- first\n- second\n\nparagraph`
  - cursor 从段落移动到第一条列表项。
  - 断言第二条列表项和 paragraph 的真实 y 不因 marker active 改变。
- [ ] 停止在 `prepend_marker_to_line()` 中直接修改 line text 作为长期路径；保留函数只作为兼容 helper 或删除。
- [ ] layout 时对 active marker 先 shape/measure marker lane，再 layout 正文 content lane。
- [ ] 正文 wrap width 不因 marker 可见性变化；marker 只影响第一行的 x 视觉偏移和 hit-test source map。
- [ ] source map 构造明确区分 marker segment 与 content segment；marker bytes 映射到 `marker_source_range`，content bytes 映射到正文 source。

**Verification:**

- Run: `cargo test -p edit-plus-markdown --lib -- list_item`
- Run: `cargo test -p edit-plus-markdown --lib -- active_marker`
- Run: `cargo test -p edit-plus-markdown --lib -- hit_test_byte_roundtrip_inside_list_item_respects_indent`

---

## Task 4: 空列表/空编号 Enter 退出列表模式

**Files:**

- Modify: `crates/markdown/src/edit.rs`
- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/app/src/dispatch/wysiwyg.rs`

**Interfaces:**

- Produces: `EditAugmentation { replace_range: Option<Range<usize>>, insert_text, cursor_byte_after }`
- Consumes: existing `AugmentKind::Enter`

- [ ] 扩展 `ui::plugin::EditAugmentation`，新增纯数据字段 `replace_range: Option<(usize, usize)>`。
- [ ] 修改 `dispatch_wysiwyg_augmented_enter()`：如果 augmentation 带 `replace_range`，先用标准编辑命令删除 range，再插入 `insert_text`，所有文本修改仍经过 `execute_edit_command_v2`。
- [ ] 写失败测试 `enter_on_empty_bullet_exits_list`：
  - source: `- `
  - cursor at end。
  - `AugmentEdit(Enter)` 返回 replace range `0..2`，insert text `""` 或 `"\n"` 按目标行为确定。
  - 最终文档为普通空段落，不再包含 `- `。
- [ ] 写失败测试 `enter_on_empty_ordered_item_exits_list`：
  - source: `1. `
  - cursor at end。
  - 最终文档不再包含 `1. `，cursor 位于段落开头。
- [ ] 写失败测试 `enter_on_non_empty_ordered_item_continues_numbering`：
  - source: `1. abc`
  - cursor at end。
  - 继续得到 `\n2. `。
- [ ] 对 task list 覆盖 `- [ ] ` 和 `- [x] `，空项回车退出 list，不保留 checkbox marker。

**Recommended semantics:**

- 单独一行空列表项：删除 marker，停留在普通空段落。
- 非空列表项末尾：继续下一项。
- 非空列表项中间：先拆分当前项，再插入下一项 marker。
- 空列表项后还有同级列表项：当前项退出为普通空段落，并保留下方列表项；不在本阶段重编号整组。

**Verification:**

- Run: `cargo test -p edit-plus-markdown --lib -- augment_edit`
- Run: `cargo test -p edit-plus-app --lib -- wysiwyg_route_maps_edit_hooks`

---

## Task 5: Fenced code block 建立真实 source map 与编辑定位

**Files:**

- Modify: `crates/markdown/src/builder.rs`
- Modify: `crates/markdown/src/layout/block.rs`
- Modify: `crates/markdown/src/layout/types.rs`
- Modify: `crates/markdown/src/view.rs`

**Interfaces:**

- Produces: code line source ranges on `BlockNode` or layout-time helper
- Produces: `source_bytes_by_visual_grapheme` for `LaidOutLine` inside `CodeBlock`
- Consumes: parser event ranges for fenced code block start/end

- [ ] 写失败测试 `code_block_hit_test_returns_code_content_byte`：
  - source:
    ```text
    ```rust
    abc
    def
    ```

    paragraph
    ```
  - render editor。
  - 点击 code line `def` 的 `e` 中点。
  - 断言 `hit_test_byte()` 返回 source 中 `def` 的 byte，而不是 paragraph byte。
- [ ] 写失败测试 `typing_inside_code_block_updates_code_block`：
  - 通过 WYSIWYG click 设置 cursor 到 code line。
  - 执行 `InsertChar('X')`。
  - 断言 `X` 出现在 fenced code 内部，不出现在下面段落。
- [ ] 在 builder 阶段记录 code block content 的每行 source start。优先使用 parser event range；若 event range 包含 fences，则 helper 需跳过 opening fence 与 closing fence。
- [ ] `CodeBlock` layout 创建 `LaidOutLine` 时填充 `source_bytes_by_visual_grapheme`。
- [ ] `find_line_idx_in_block()` 对 code block 使用 source line ranges，而不是只依赖 style spans 或 single-line fallback。
- [ ] 对 empty code line 生成 sentinel map，保证点击空行能落到该行开始 byte。

**Verification:**

- Run: `cargo test -p edit-plus-markdown --lib -- code_block`
- Run: `cargo test -p edit-plus-app --lib -- wysiwyg`

---

## Cross-Task Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test -p edit-plus-markdown --lib -- preedit`
- [ ] `cargo test -p edit-plus-markdown --lib -- active_marker`
- [ ] `cargo test -p edit-plus-markdown --lib -- list_item`
- [ ] `cargo test -p edit-plus-markdown --lib -- code_block`
- [ ] `cargo test -p edit-plus-app --lib -- ime`
- [ ] `cargo test -p edit-plus-app --lib -- wysiwyg`
- [ ] `cargo test -p edit-plus-app --lib -- mouse`
- [ ] `cargo check -p edit-plus-app`
- [ ] 重大修改完成后执行 `./scripts/verify.sh`

---

## Manual Acceptance

- [ ] 在 mdeditor 中输入中文 IME preedit，preedit 出现在 Markdown 光标处，候选窗跟随 preedit cursor，后面的文字实时右移或重排。
- [ ] 在窄宽度下 preedit 足够长时，当前行重新 wrap，preedit 后面的内容进入下一视觉行而不是重叠。
- [ ] 光标在普通段落、标题、列表、编号、代码块内都垂直居中，高度一致；单击不留下空 selection。
- [ ] 点击 `12. abc` 的编号、点、空格和正文时，光标不会卡在数字中间，左右移动按 grapheme/source boundary 前进。
- [ ] 在 `1. ` 后不输入内容直接回车，列表退出为普通段落；`- `、`- [ ] `、`- [x] ` 行为一致。
- [ ] 在 fenced code block 中点击并输入，文字进入 code block 对应行，不进入下方段落。

---

## Implementation Order

1. 先做 Task 5 code block source map。它是最独立的 bug，能快速降低“输入到下方段落”的风险。
2. 再做 Task 4 空列表/编号 Enter。它是编辑命令语义，不依赖 preedit segment。
3. 再做 Task 3 marker lane。它会影响列表/编号光标和 y 稳定性，应在 Enter 语义明确后处理。
4. 再做 Task 2 二阶段点击和 selection 清理。marker lane 落地后，二阶段命中才有稳定目标。
5. 最后做 Task 1 preedit layout。它影响最大，涉及 app/plugin/markdown 三层，必须在 source map 和 marker 基础稳定后做。

这个顺序刻意先修“真实 source byte 映射”和“编辑命令语义”，再处理视觉动态布局，避免 IME/preedit 修复建立在不稳定的 hit-test 和 marker map 上。

---

## Risks

- `EditAugmentation` 增加 replace range 会触及 app 编辑命令链路，必须确保 undo/redo、render cache invalidation 仍由 `execute_edit_command_v2` 驱动。
- marker lane 若只改 layout 不改 hit-test，会出现“看起来对，点起来错”的半修状态；Task 3 必须同时更新 source map 测试。
- preedit segment 如果映射成真实 source byte，会污染编辑命令；必须作为 transient segment，commit 时仍由 IME commit 写入 `DocumentView`。
- code block source range 依赖 parser event range。如果 event range 对 fenced block 的 opening/closing fence 语义不稳定，需要先补 parser-level tests 固化契约。

---

## Self-Review

- 范围覆盖了用户报告的 5 个问题：IME preedit、光标/点击、编号/list marker、空列表回车、code block 输入目标。
- 每个阶段都有失败测试、修改文件、接口和验证命令。
- 计划遵守分层红线：新增跨层内容只放在 `ui::plugin` 纯数据协议；app 不把状态塞给 ui，markdown 不依赖 app。
- 没有要求一次性大重构；每个任务可以独立实现和回滚。
