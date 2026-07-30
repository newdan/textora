# Mdeditor WYSIWYG 编辑视图修复方案

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 Markdown WYSIWYG 编辑视图中 inline span、标题/区块编辑态、列表布局稳定性和滚动条交互失效的问题。

**Architecture:** 保持现有分层：`app` 层负责输入、光标、编辑命令和滚动调度；`markdown` 插件负责源码到视觉布局的映射、块/行 materialize、命中检测和 WYSIWYG 光标绘制；`ui` 只暴露纯数据协议。核心改法是把“活动编辑区域”从 inline span 扩展为统一的 active block/active inline 状态，并让所有 plugin 自渲染视图共享一条滚动调度路径。

**Tech Stack:** Rust workspace；`edit-plus-markdown` 负责 parser/builder/layout/view；`edit-plus-app` 负责 dispatch；`edit-plus-ui` 负责 `ViewPlugin` 协议和 scrollbar widget；验证使用 `cargo test`、`cargo check`、`cargo fmt`，重大修改后跑 `./scripts/verify.sh`。

## Global Constraints

- 全程遵守 `AGENTS.md`：中文沟通、先复现测试再修、同一 bug 修改超过两次必须回到根因重审。
- 单阶段修改超过 3 个文件时必须拆分为子任务；每次提交前必须确保编译通过。
- 绝对禁止让 `crates/ui` 依赖或访问 `crates/app` 状态结构体。
- 新增跨层数据只能放在 `crates/ui/src/plugin.rs` 的纯数据协议，或 `crates/markdown` 内部纯数据结构中。
- WYSIWYG 光标移动不应触发整篇 Markdown 全量 parse/build；源码变化可以继续走现有全量 parse/build。
- 列表、标题、引用等 block marker 进入编辑态时不得改变后续 block 的 `y`，除非用户实际编辑源码导致内容换行。

---

## 当前证据

- Inline span 纯路径已经接入：`crates/markdown/src/edit.rs:84` 的 `materialize_line()` 会在光标落入 `StyleSpan::source_range` 时展开源码 marker。
- 已运行验证：`cargo test -p edit-plus-markdown --lib -- editor_render_expands_cursor_span_source_markers` 通过。这说明“直接发送 `SetCursorByte` 后加粗 span 展开”不是当前主因。
- 文本块和列表项都在 layout 阶段调用 `materialize_line()`：`crates/markdown/src/layout/block.rs:225`、`crates/markdown/src/layout/block.rs:430`。
- 标题使用 `BlockKind::Heading` 进入 `layout_text_block()`，但标题自身没有 block marker materialize；`# ` 不属于 inline `StyleSpan`。
- 鼠标点击 WYSIWYG 时先用旧 layout 做 `HitTestByte`，再发送 `SetCursorByte`：`crates/app/src/dispatch/mouse.rs:99` 到 `crates/app/src/dispatch/mouse.rs:124`。
- 光标移动后只 invalidates 涉及 byte 的 block，再 `ensure_visible()` 重排：`crates/markdown/src/view.rs:435` 到 `crates/markdown/src/view.rs:454`。若活动行展开后高度变化，后续 block 的 `y_delta` 会被传播。
- `MarkdownEditorView` 同时声明 `allows_editing() == true` 和 `is_wysiwyg() == true`：`crates/markdown/src/view.rs:1455`、`crates/markdown/src/view.rs:1470`。
- 滚轮的 plugin 滚动路径排除了所有 `allows_editing()` 插件：`crates/app/src/app_scroll.rs:86`、`crates/app/src/app_scroll.rs:159`。WYSIWYG 渲染走 plugin scroll，但滚轮会落回普通 `DocumentView` viewport：`crates/app/src/app_scroll.rs:320` 到 `crates/app/src/app_scroll.rs:354`。
- Scrollbar 的 drag 会转换为 `UpdateScrollTop`，但 PageUp/PageDown 仍按 `DocumentView` viewport 派发：`crates/app/src/events.rs:372` 到 `crates/app/src/events.rs:382`。

## 根因分析

### 1. 进入加粗时 span 恢复到编辑状态不稳定

直接 `SetCursorByte` 的单元测试已通过，说明 inline span 展开函数和 render 后状态本身可用。交互层仍可能失败的根因是点击流程使用“折叠态旧布局”命中一次后就结束：点击到 `world` 时返回的是折叠文本映射出来的源码 byte，然后下一帧才展开 `**world**`。如果用户点击位置接近 marker 边界，或当前行刚因进入编辑态改变 wrap，第一次命中的 byte 与展开后的视觉位置不是同一个插入点。

修改方向：WYSIWYG 点击进入 inline span 后执行二阶段命中。第一阶段用当前 layout 得到候选 byte；发送 `SetCursorByte` 并强制对候选 block 做一次同步 materialize；第二阶段在新 layout 上重新 `HitTestByte`，把最终 cursor 写回 `DocumentView` 和 plugin。

### 2. 光标进入标题或其他区块时没有反应

当前 `EditContext` 只描述 `cursor_byte`，`materialize_line()` 只展开 inline `StyleSpan`。标题的 `# `、列表的 `- ` / `1. `、引用的 `> `、任务列表的 `[ ]` 是 block marker，不在 `StyleSpan` 里。光标进入这些 block 后，cursor byte 已改变，但布局文本仍是折叠后的 block 内容，所以视觉上“没有反应”。

修改方向：新增 block 级活动编辑模型，而不是把 block marker 硬塞进 inline span。`markdown` 内部用纯数据表示 `ActiveEditRegion { block_source_range, cursor_byte }`，layout 时判断当前 block 是否 active；active block 的 marker 以独立 marker segment 绘制和命中，不参与正文 wrap 高度计算。

### 3. 列表进入编辑状态时下面行的 y 被改变

列表项当前在 layout 阶段用 `ctx.wrap_text(&materialized.text, font_size)` 计算行数：`crates/markdown/src/layout/block.rs:240`。如果为了编辑态把 marker 拼进正文，文本宽度和 wrap 行数会变化，`content_h` 随之变化，`LazyLayout` 的 `y_delta` 会把变化传播到下面 block。现有 cursor-only 重排路径也会更新 `content_height`：`crates/markdown/src/view.rs:453` 到 `crates/markdown/src/view.rs:454`。

修改方向：列表 marker 和任务 marker 不进入正文 wrap；它们作为 active block marker 画在固定 marker lane 中。正文仍用折叠后的 content text 测量高度，只有用户真实编辑导致正文内容变化时才允许后续 block y 改变。

### 4. 滚动条失效

WYSIWYG Markdown 是 plugin 自渲染视图，render 使用 `PreviewEngine::scroll_y`。但是滚轮调度中的 `plugin_scroll_by_pixels()` 对 `allows_editing()` 直接返回 `NONE`，导致 WYSIWYG 滚轮落回普通 `DocumentView` viewport，两个滚动状态分裂。Scrollbar thumb drag 走 `UpdateScrollTop` 时能命中 plugin；track PageUp/PageDown 和 wheel 则不统一，所以表现为滚动条/滚动失效或状态不同步。

修改方向：把滚动判断从 `!allows_editing()` 改为 `handles_own_rendering()` 或新增更明确的 `uses_plugin_scroll()`。凡是 plugin 自渲染，包括 WYSIWYG editor、preview、novel，都用 `PluginMessage::Scroll/SetScrollY/SetScrollRatio` 更新同一个 `scroll_y`。

## 分阶段修改方案

### Task 1: 补交互层 inline span 二阶段命中测试

**Files:**
- Modify: `crates/app/src/dispatch/mouse.rs`
- Test: `crates/app/src/dispatch_boundary_tests.rs` 或新增 `crates/app/src/dispatch/wysiwyg_tests.rs`
- Test support: `crates/markdown/src/view.rs`

**Steps:**
- [ ] 写失败测试：构造 Markdown WYSIWYG 文档 `hello **world** here`，模拟点击折叠态 `world` 中部，断言点击后 `FlatLines` 包含 `hello **world** here`，且 cursor byte 与二次 `HitTestByte` 一致。
- [ ] 在 `dispatch_editor_mouse_input()` 的 WYSIWYG 分支中抽出 `set_wysiwyg_cursor_from_point(px, py)`，封装 source sync、初次命中、`SetCursorByte`、局部重排、二次命中。
- [ ] 在 markdown plugin 内提供同步查询入口，例如 `PluginQuery::RefreshWysiwygLayoutForCursor`, 或复用现有 `SetCursorByte` 后的 render 前布局刷新函数；协议必须是纯数据。
- [ ] 运行 `cargo test -p edit-plus-app --lib -- wysiwyg` 和 `cargo test -p edit-plus-markdown --lib -- editor_render_expands_cursor_span_source_markers`。

### Task 2: 增加 block marker 活动编辑模型

**Files:**
- Modify: `crates/markdown/src/edit.rs`
- Modify: `crates/markdown/src/layout/block.rs`
- Modify: `crates/markdown/src/layout/types.rs`

**Steps:**
- [ ] 在 `edit.rs` 新增 `ActiveBlockMarker`、`MaterializedBlockLine`，字段包含 `marker_text`、`marker_source_range`、`content_text`、`content_source_map`。
- [ ] 在 `layout/types.rs` 提供 `find_active_block_for_byte(byte)`，返回最内层 block 及 line index，覆盖 heading、list item、blockquote、task list。
- [ ] 在 `layout/block.rs` 中让 heading/list/blockquote 使用 active marker lane：正文 wrap 仍基于 content text，marker 独立生成 rect 和 source map。
- [ ] 写失败测试：`# Title` 中 cursor 位于标题 source range 时，FlatLines 或新增 debug query 能看到 active marker `# `；引用和任务列表同理。
- [ ] 运行 `cargo test -p edit-plus-markdown --lib -- heading` 和 `cargo test -p edit-plus-markdown --lib -- list`.

### Task 3: 保持列表进入编辑态时 y 稳定

**Files:**
- Modify: `crates/markdown/src/layout/block.rs`
- Modify: `crates/markdown/src/layout/types.rs`
- Modify: `crates/markdown/src/view.rs`

**Steps:**
- [ ] 写失败测试：两条列表 `- first\n- second`，把 cursor 从正文移入第一条 marker/source range，断言第二条 `LaidOutBlock` 的 `rect.y` 不变。
- [ ] 调整 cursor-only invalidation：如果 cursor 移动只改变 marker 可见性，更新 marker draw/source map，不更新正文 wrap 计数和 block height。
- [ ] 保留真实编辑后的重排：当 `UpdateSource` generation 改变时仍走 source dirty，全量 rebuild 可改变 y。
- [ ] 运行 `cargo test -p edit-plus-markdown --lib -- list_item` 和 `cargo test -p edit-plus-markdown --lib -- lazy_layout_y_delta_propagates_correctly`。

### Task 4: 统一 WYSIWYG 与 preview/novel 的 plugin 滚动路径

**Files:**
- Modify: `crates/app/src/app_scroll.rs`
- Modify: `crates/app/src/dispatch/viewport.rs`
- Modify: `crates/app/src/events.rs`

**Steps:**
- [ ] 写失败测试：active tab 使用 `is_wysiwyg() == true` 的 stub plugin，调用 `handle_scroll(PixelDelta)` 后断言 plugin `ScrollY` 增加，`DocumentView.display.viewport.scroll_top` 不变。
- [ ] 把 `plugin_scroll_by_command()` 和 `plugin_scroll_by_pixels()` 的 early return 从 `tab.plugin.allows_editing()` 改为 `!tab.plugin.handles_own_rendering()`。
- [ ] 修改 scrollbar PageUp/PageDown 翻译：对于 `handles_own_rendering()` 的 active tab，派发 plugin scroll command 或 `UpdateScrollTop`，不要读取 `DocumentView` viewport height。
- [ ] 确认 `dispatch_viewport_action(UpdateScrollTop)` 继续使用 plugin `ContentHeight/ScrollY`，并补 WYSIWYG case 测试。
- [ ] 运行 `cargo test -p edit-plus-app --lib -- plugin_scroll` 和 `cargo test -p edit-plus-app --lib -- scrollbar`。

## 验证矩阵

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test -p edit-plus-markdown --lib -- wysiwyg`
- [ ] `cargo test -p edit-plus-markdown --lib -- list`
- [ ] `cargo test -p edit-plus-app --lib -- plugin_scroll`
- [ ] `cargo test -p edit-plus-app --lib -- scrollbar`
- [ ] `cargo check -p edit-plus-app`
- [ ] 重大修改完成后执行 `./scripts/verify.sh`

## 手动验收

- [ ] 打开包含 `hello **world** here` 的 `.md`，点击 `world`，第一次点击后就显示 `**world**` 并能在 marker 内移动光标。
- [ ] 点击 `# Title` 正文或 marker 附近，标题进入 block 编辑态，`# ` 可见，光标位置正确。
- [ ] 点击列表第一项进入编辑态，第二项和后续段落的 y 坐标不跳动。
- [ ] 鼠标滚轮、滚动条拖拽、滚动条 track PageUp/PageDown 在 WYSIWYG Markdown 中都滚动同一个视图，thumb 位置同步。
