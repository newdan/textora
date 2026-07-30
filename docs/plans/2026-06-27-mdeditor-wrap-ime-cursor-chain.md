# Mdeditor Soft Wrap IME Cursor Chain Fix Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:systematic-debugging` first, then `superpowers:test-driven-development`, then `superpowers:subagent-driven-development` or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 Markdown WYSIWYG 编辑态中软折行后光标位置错误、非首屏光标/IME 锚点错误、preedit 不显示，以及 IME commit 后文字落到视觉光标前一个字的问题。

**Architecture:** 保持现有分层：`app` 层负责窗口事件、IME 状态、`DocumentView` 写入和 plugin 同步；`markdown` plugin 负责 Markdown source 到视觉 flat line 的投影、软折行 hit-test、cursor rect 和 WYSIWYG 自绘光标；`ui` 只承载纯数据协议。修复方向不是补多个偏移量，而是建立一条统一坐标链路：`SourceByte -> VisualGraphemePos -> PluginDocumentRect -> WindowRect`，并让点击、导航、IME preedit 绘制、候选窗定位和 commit 后同步全部复用这条链路。

**Tech Stack:** Rust workspace；`edit-plus-app` 的 `app_lifecycle` / `app_renderer` / `app_window` / `dispatch`；`edit-plus-markdown` 的 `view` / `layout` / `grapheme_map`；`edit-plus-ui` 的 plugin 纯数据协议；验证使用 `cargo test`、`cargo check`、`cargo fmt`，重大修改后执行 `./scripts/verify.sh`。

## Global Constraints

- 全程遵守 `AGENTS.md`：中文沟通，遇 bug 先写复现测试再修，同一 bug 修改超过两次必须回到根因重审。
- 本计划执行阶段每个任务修改文件数控制在 3 个以内；每次提交前必须确保编译通过。
- `crates/ui` 只能定义纯数据协议，绝对禁止依赖或访问 `crates/app` 状态结构体。
- 用户可见的 WYSIWYG cursor byte 必须落在 UTF-8 char boundary 和 UAX #29 grapheme boundary。
- 软折行后，视觉行的 `x/y`、source byte map、hit-test、cursor rect 必须互相 roundtrip。
- WYSIWYG 自渲染视图不得依赖普通编辑器的 `cursor_render_state` 来绘制 preedit。
- IME commit 后必须立即回读 `DocumentView` snapped cursor byte，再同步给 plugin。

---

## 当前证据

### 1. WYSIWYG 光标和 hit-test 已在 plugin 内部走 flat line/source map

- `crates/markdown/src/view.rs` 的 `cursor_screen_pos()` 通过 `find_flat_and_grapheme_for_byte(cursor_byte)` 找到 `flat_idx + visual_grapheme`，再用 `flat_line.rect.x + grapheme_x()` 生成 cursor rect。
- `hit_test_byte()` 使用 `doc_y = y - offset_y`，再在 `lazy.flat_lines` 中找 `rect.y` 命中的视觉行，最后用 `byte_from_flat_line_and_visual_grapheme()` 回到 source byte。
- `crates/markdown/src/layout/types.rs` 已有 `FlatLineSourceMap.source_bytes_by_visual_grapheme`，说明 grapheme 化已经部分落地。
- `crates/markdown/src/layout/block.rs` 的 `layout_line_with_styles()` 会按 wrapped segment 切分 materialized source map。

### 2. WYSIWYG 渲染和 app 层 IME/preedit 走了两套路径

- `crates/app/src/app_renderer.rs` 的 plugin 自渲染分支会调用 `tab.plugin.render(...)`，然后跳过普通编辑器内容。
- 普通编辑器 preedit 绘制位于 `app_renderer.rs` 的非 plugin `else` 分支：当 active tab 是 WYSIWYG plugin 时，这段不会执行。
- `MarkdownEditorView::render()` 只绘制 WYSIWYG 光标，没有绘制 `preedit_text`。
- 因此“mdeditor 状态下 IME preedit 未出现”的一阶根因很可能是 preedit 顶点根本没有进入 WYSIWYG 渲染路径。

### 3. WYSIWYG 候选窗定位与渲染光标可能使用不同偏移

- `MarkdownEditorView::render()` 绘制光标时使用 `bounds.x + cx`、`bounds.y + cy`。
- `App::plugin_render_bounds()` 是 plugin 主渲染位置的事实来源，包含 editor rect、TOC offset、reading column 居中、top padding。
- `App::update_ime_cursor_area()` 查询 WYSIWYG `CursorScreenPos` 后只计算 `cursor_x = x + preedit_advance_px` 和 `cursor_y = content_top + y + h`，没有直接复用 `plugin_render_bounds()`。
- 非首屏或 reading column 居中时，如果 plugin 返回的是文档坐标而 app 当成屏幕坐标，候选窗和 preedit 锚点会漂移。

### 4. IME commit 绕过了普通编辑命令后的 WYSIWYG 即时同步保障

- 普通 `dispatch/editor.rs` 在编辑命令结束后会判断 `tab.plugin.is_wysiwyg()` 并调用 `sync_wysiwyg_plugin_state()`。
- `WindowEvent::Ime(Ime::Commit)` 在 `app_lifecycle.rs` 中手动循环 `execute_edit_command_v2(InsertChar)`，随后更新普通 display/cache 状态。
- 当前 commit 路径需要纳入同一条 WYSIWYG 同步：写入后立即回读 `DocumentView` snapped cursor byte，再 `UpdateSource + SetCursorByte` 给 plugin，避免下一帧前 plugin 仍持旧 cursor/source 投影。

### 5. 软折行仍是高风险点

- `wrap_text()` 复用 `ui::layout::compute_visual_lines()`，每个 `WrappedLine` 有 `byte_start/byte_end`。
- `layout_line_with_styles()` 用 `grapheme_index_at_byte(materialized_text, seg_start/seg_end)` 切分 full source map。
- `find_flat_and_grapheme_for_byte()` 遇到 wrapped segment sentinel 重复时会跳过非最后 segment 的 sentinel。
- 这些机制方向正确，但还缺少覆盖“同一 source 行软折成多条 flat line + cursor 在第二/第三视觉行 + 非零 scroll_y + IME preedit/commit”的端到端测试。

## 根因假设

### Hypothesis A: WYSIWYG preedit 不显示是渲染路径缺失

当前 preedit 文本只在普通编辑器渲染分支用 `render_pipeline::preedit_text_vertices()` 绘制。WYSIWYG 是 plugin 自渲染分支，`MarkdownEditorView` 又只画 cursor，不画 preedit。因此 composition 期间 app 有 `preedit_text`，但画面没有 preedit。

### Hypothesis B: 非首屏光标/候选窗错位是坐标空间混用

`CursorScreenPos` 的注释写着 screen pixel rect，但实现返回的是 plugin document coordinates：`fl.rect.x/y`。WYSIWYG render 再加 `bounds.x/y`。IME candidate path 却没有统一复用 `plugin_render_bounds()`，很容易出现 `content_top`、preview top pad、reading column x、scroll_y 的重复或缺失。

### Hypothesis C: 软折行后输入落在前一个字是 source byte 投影在 wrap 边界处选错 segment

软折行后，同一 source 行拆成多个 flat line。若 `find_flat_and_grapheme_for_byte()` 在 segment boundary、marker expansion、或 sentinel 重复时选择了上一视觉行的尾部 byte，视觉光标会画在看似正确的位置附近，但 `DocumentView` cursor byte 会在前一个 grapheme。IME commit 忠实插入到该 byte，于是用户看到文字进入光标前一个字。

### Hypothesis D: commit 后 plugin 同步时机不一致会放大错位

IME commit 直接写 `DocumentView`，如果未立即同步 plugin source/cursor，下一次 `CursorScreenPos`、preedit candidate area、或者下一次 hit-test 可能使用 stale flat line/source map。这个问题在非首屏和软折行处更明显。

## Target Model

统一定义四个坐标空间，禁止模糊命名：

```text
SourceByte
  absolute UTF-8 byte offset in Markdown source

VisualGraphemePos
  flat_line_idx + line-local visual grapheme index

PluginDocumentRect
  x/y/w/h inside plugin document coordinate space
  y is before subtracting scroll_y if flat_lines are stored in document coordinates

WindowRect
  physical pixel rect in the winit window
  equals plugin_render_bounds().origin + PluginDocumentRect - plugin scroll projection
```

所有链路只允许走下面的投影：

```text
SourceByte
  -> VisualGraphemePos
  -> PluginDocumentRect
  -> WindowRect
```

反向命中只允许走：

```text
WindowPoint
  -> PluginDocumentPoint
  -> VisualGraphemePos
  -> SourceByte
  -> DocumentView snapped SourceByte
```

## 修复设计

### 1. 明确 plugin cursor rect 坐标协议

`PluginQuery::CursorScreenPos` 当前命名容易误导。执行阶段优先不破坏协议，先在 `ui/src/plugin.rs` 文档中明确：WYSIWYG 返回 plugin document coordinates；app 必须使用 `plugin_render_bounds()` 转换到 window coordinates。

如果后续需要更清晰的协议，新增纯数据查询：

```rust
PluginQuery::CursorDocumentRect(usize)
PluginResponse::CursorDocumentRect(Option<(f32, f32, f32, f32)>)
```

保留旧 `CursorScreenPos` 作为兼容入口，内部转发，避免一次性改大面积调用。

### 2. 新增 app 层统一转换函数

在 `crates/app/src/app_window.rs` 或更合适的 app helper 中新增只读 helper：

```rust
fn wysiwyg_cursor_window_rect(&self, cursor_byte: usize) -> Option<ui::core::geom::Rect>
```

职责：

- 获取 `plugin_render_bounds()`。
- 查询 plugin cursor document rect。
- 把 document rect 转为 window rect：`bounds.x + rect.x`、`bounds.y + rect.y`。
- 只在这里处理 preedit candidate 的 advance 偏移。
- 不读取普通编辑器 `cursor_render_state`。

### 3. WYSIWYG 自渲染分支绘制 preedit

在 plugin 自渲染分支完成 `deferred_preview_verts` 后，如果 active plugin 是 WYSIWYG 且 `preedit_text` 非空：

- 查询统一的 `wysiwyg_cursor_window_rect()`。
- 用现有 `render_pipeline::preedit_text_vertices()` 绘制 preedit 文本。
- preedit 的 y 使用 cursor rect top，而候选窗可使用 rect bottom；两者不要混用。
- 搜索框聚焦时继续由 search bar 自己处理 IME，不进入 WYSIWYG preedit。

此任务不要求 markdown plugin 直接知道 preedit text，避免把 app 输入状态塞进 plugin。

### 4. IME commit 后纳入 WYSIWYG 同步

在 `WindowEvent::Ime(Ime::Commit)` 的文档插入路径后：

- 清空 `preedit_text` / `preedit_cursor`。
- 回读 `dv.cursor_offset()`。
- 对 WYSIWYG active plugin 调用 `sync_wysiwyg_plugin_state()`。
- 重置 WYSIWYG preferred x，避免 commit 后上下移动沿用旧 visual x。
- 触发 redraw。

长期更干净的方向是把 IME commit 转成普通 `EditCommand::InsertChar` 派发，复用 `dispatch/editor.rs` 的所有后置同步；但第一阶段先做最小修复，降低行为面。

### 5. 补软折行 roundtrip 和非首屏测试

`markdown` 层负责证明 source byte 与 soft-wrapped flat line roundtrip：

- 窄 viewport 下同一段文字拆出至少 3 条 `flat_lines`。
- cursor 在第二条或第三条 flat line 中间。
- `cursor_screen_pos()` 返回的 y 必须落在对应 flat line。
- 对 cursor rect 中点执行 `hit_test_byte()`，必须得到同一个 source byte。

`app` 层负责证明窗口坐标转换和 preedit 绘制：

- 构造 WYSIWYG stub plugin，返回固定 cursor document rect。
- 设置非零 `plugin_render_bounds().x/y` 场景。
- `update_ime_cursor_area()` 或 helper 输出的 window rect 必须包含 bounds 偏移。
- plugin 自渲染分支下 `preedit_text` 非空时，顶点数量增加，且 x/y 接近 WYSIWYG cursor window rect。

## 分阶段任务

### Task 1: 补 Markdown 软折行 cursor roundtrip 复现测试

**Files:**
- Modify: `crates/markdown/src/view.rs`

**Steps:**
- [ ] 新增测试 helper：允许用窄 bounds 渲染 `MarkdownEditorView`，例如 width 160px，确保长行发生软折行。
- [ ] 新增测试 `wysiwyg_cursor_roundtrips_on_second_soft_wrapped_line`：
  - source 使用一行足够长的中英文混排 Markdown。
  - render 后断言 `flat_lines.len() >= 3`。
  - 选取第二条 flat line 中间的 source byte，通过 `SetCursorByte` 设置 cursor。
  - 再 render，调用 `cursor_screen_pos()`。
  - 断言 cursor y 落在第二条 flat line 的 rect 范围。
  - 用 cursor rect 中点调用 `hit_test_byte()`，断言返回 snapped 后同一 byte。
- [ ] 新增测试 `wysiwyg_cursor_roundtrips_after_scroll_y`：
  - 设置 `engine.scroll_y` 为非零。
  - 用窗口坐标调用 `hit_test_byte(x, y, offset_x, offset_y)`。
  - 断言 `doc_y = y - offset_y` 后仍命中正确 flat line。

**Expected before fix:** 至少一个测试暴露 y/byte roundtrip 不一致，或确认 markdown 层已正确，把问题收窄到 app 坐标转换。

### Task 2: 补 app 层 WYSIWYG cursor window rect 转换测试

**Files:**
- Modify: `crates/app/src/app_window.rs`
- Modify: `crates/app/src/app_tests.rs`

**Steps:**
- [ ] 新增 WYSIWYG stub plugin：`is_wysiwyg() == true`、`handles_own_rendering() == true`，`CursorScreenPos` 返回固定 document rect。
- [ ] 新增测试 `wysiwyg_cursor_window_rect_adds_plugin_render_bounds`：
  - 构造非零 editor rect / content top / TOC 或 reading column offset。
  - cursor document rect 为 `(10, 20, 2, 18)`。
  - 断言 helper 返回 `plugin_render_bounds().x + 10`、`plugin_render_bounds().y + 20`。
- [ ] 新增测试 `ime_cursor_area_uses_wysiwyg_window_rect`：
  - 不直接依赖 OS window，可测试纯 helper。
  - 断言候选窗 y 使用 cursor bottom，preedit 绘制 y 使用 cursor top。

**Expected before fix:** 当前逻辑只加 `content_top`，无法通过 bounds offset 测试。

### Task 3: 在 WYSIWYG 自渲染分支绘制 preedit

**Files:**
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/app_window.rs`

**Steps:**
- [ ] 新增 app 渲染单元测试或 focused helper 测试：active plugin 为 WYSIWYG、`preedit_text = "ni"`、cursor rect 可解析时，preedit vertices 不为空。
- [ ] 从普通编辑器 preedit 绘制逻辑中抽出坐标无关 helper，避免复制 shaping 和 vertex 参数。
- [ ] plugin 自渲染分支在 preview text vertices 之后、chrome 之前追加 WYSIWYG preedit vertices。
- [ ] 搜索框聚焦时保持 search bar preedit 优先，不绘制 WYSIWYG preedit。

**Expected after fix:** mdeditor 状态下 composition 期间 preedit 可见，并跟随 WYSIWYG 光标。

### Task 4: IME commit 后立即同步 WYSIWYG plugin

**Files:**
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/app_tests.rs`

**Steps:**
- [ ] 新增测试 `ime_commit_syncs_wysiwyg_source_and_snapped_cursor`：
  - active tab 使用 WYSIWYG recording plugin。
  - 初始 cursor 在某个 source byte。
  - 模拟 commit 插入中文字符。
  - 断言 plugin 收到 `UpdateSource` 和 `SetCursorByte(snapped_after_insert)`。
- [ ] 在 `Ime::Commit` 文档插入路径后调用统一 WYSIWYG sync。
- [ ] commit 后清空 `preedit_text` / `preedit_cursor`，并重置 `wysiwyg_preferred_x`。
- [ ] 保持普通编辑器 display map/render cache 更新逻辑不变。

**Expected after fix:** commit 后 plugin 的 source/cursor 不等下一帧才追上，后续输入不会沿用旧位置。

### Task 5: 收紧 wrap segment source map 边界

**Files:**
- Modify: `crates/markdown/src/layout/block.rs`
- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/markdown/src/grapheme_map.rs`

**Steps:**
- [ ] 若 Task 1 暴露边界错误，新增最小纯函数测试覆盖 `map[g_start..=g_end]` 的 segment 切片。
- [ ] 明确 wrapped segment 采用半开 source byte range `[byte_start, byte_end)`，source map 采用 `grapheme_count + 1` sentinel。
- [ ] 对 segment 末尾 sentinel 与下一 segment 首位重复的情况保留“视觉行 affinity”：普通 cursor 在行尾时属于当前行，向右移动才进入下一行；点击下一行开头时属于下一行。
- [ ] 将 `find_flat_and_grapheme_for_byte()` 的 nearest fallback 限制到同一 source line 或最近 visual line，避免跨 soft-wrap/段落抢到前一个字。

**Expected after fix:** 软折行边界处 cursor 绘制、hit-test 和 commit byte 一致。

## 验证矩阵

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test -p edit-plus-markdown --lib -- wysiwyg_cursor_roundtrips`
- [ ] `cargo test -p edit-plus-markdown --lib -- hit_test_byte_roundtrip`
- [ ] `cargo test -p edit-plus-app --lib -- wysiwyg_cursor_window_rect`
- [ ] `cargo test -p edit-plus-app --lib -- ime_commit_syncs_wysiwyg`
- [ ] `cargo test -p edit-plus-app --lib -- preedit`
- [ ] `cargo check -p edit-plus-app`
- [ ] 重大修改完成后执行 `./scripts/verify.sh`

## 手动验收

- [ ] 打开一篇 Markdown，窗口调窄到同一段落软折成多行。
- [ ] 在第二条及第三条软折视觉行点击，光标 y 与点击视觉行一致。
- [ ] 在软折行中间输入中文，preedit 立即显示在 WYSIWYG 光标处。
- [ ] commit 后中文插入到视觉光标位置，不落到前一个字。
- [ ] 滚动到非首屏后重复点击、preedit、commit，候选窗和 preedit 均跟随当前光标。
- [ ] 在 bold/code/link marker 展开态、heading/list/blockquote marker 展开态重复上述流程。

## 非目标

- 不在本阶段重构整个 plugin 协议。
- 不把 `preedit_text` 存进 markdown plugin。
- 不改变普通编辑器 IME 行为。
- 不重新设计 Markdown block lazy layout。

## 自查

- 本计划只新增方案文档，不修改业务代码。
- 修复任务先写复现测试，符合 `AGENTS.md` 的根因分析要求。
- 跨层边界仍然由 `ui::plugin` 纯数据协议承载，未让 `ui` 依赖 `app`。
- 方案覆盖用户报告的四个症状：软折行光标、非首屏光标、preedit 不显示、commit 插入到前一个字。
