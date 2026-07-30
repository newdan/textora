# WYSIWYG 编辑链路系统性梳理与根因方案（空行 / 光标 / 输入刷新 / InsertChar 增强）

> **✅ 已落地（2026-07-06）**：L1 → L4 全部阶段已按方案执行完毕，见提交
> `33d38efa`（L1）· `6cbc1bc3`（L2）· `abefce76`（L3-3a）· `24c2bdd6`（L3-3b）·
> `c995485f`（L4-4a）· `ce2e4ca8`（L4-4b）。
>
> 目标：把散落在 `markdown/src/view.rs`、`layout/types.rs`、`dispatch/wysiwyg.rs`、`dispatch/editor.rs`、`app_renderer.rs` 里的 WYSIWYG 编辑路径重新梳理清楚，找出反复被"打补丁"的地方，给出一次性根治的分层方案。
>
> 前置阅读：
> - `docs/plans_wysiwyg_enter_fix.md`（本次前的 Enter 修复方案）
> - `docs/plans/2026-07-01-wysiwyg-enter-behavior-fix.md`
> - `docs/plans/2026-07-03-wysiwyg-cursor-path-convergence.md`

---

## 一、链路全景（现状）

WYSIWYG 从"按键"到"落到屏幕上"的完整链路：

```
KeyboardInput / Ime::Commit
  └─► App::dispatch_edit_command(EditCommand, event_loop)
        │  (crates/app/src/dispatch/editor.rs:87)
        │
        ├─ plugin.allows_editing() ? 否 → 只允许导航
        ├─ plugin.handles_own_rendering() ? 是 → 走 WYSIWYG 增强路径
        │
        └─► wysiwyg_route_for_command(cmd)  (editor.rs:18)
              │
              ├─ 导航（MoveLeft/Right/Up/Down/PageUp/PageDown/Extend*）
              │   → dispatch_wysiwyg_navigation
              │     → plugin.query(VisualMove) → set_wysiwyg_cursor_and_selection
              │
              ├─ AugmentedEnter (InsertNewline)
              │   → dispatch_wysiwyg_augmented_edit(AugmentKind::Enter, ..)
              │
              ├─ AugmentedBackspace (Backspace)
              │   → dispatch_wysiwyg_augmented_edit(AugmentKind::Backspace, ..)
              │
              └─ AugmentedInsertText (InsertChar/InsertText)
                  → dispatch_wysiwyg_augmented_edit(AugmentKind::InsertText(s), ..)

dispatch_wysiwyg_augmented_edit  (wysiwyg.rs:322)
  1. cur = doc.cursor_offset()
  2. aug = plugin.augment(current_byte, kind)
       └─► MarkdownEditorView::augment_edit
             └─► augment_enter / augment_backspace / augment_insert_text
                   └─► classify_enter_context → EnterContext (10+ 类别)
                       └─► *_augmentation(...) → EditAugmentation { replace_range, insert_text, cursor_byte_after }
  3. if aug: position_document_for_wysiwyg_replace_range(range)
             execute_augmentation_text_change → InsertText | DeleteRange | fallback
             if cursor 不等: cursor_move_to_offset(cursor_byte_after)
  4. sync_plugin_state()  (每次 render 也会再跑一次)
```

渲染侧：

```
render()  (app_renderer.rs:319)
  ├─ sync_plugin_state()  ← 每帧都跑
  │    ├─ PluginQuery::NeedsSourceUpdate(generation) → 若 dirty 就 rebuild source string
  │    ├─ 双向对齐 selection: dv.selection_range ↔ plugin.sel
  │    ├─ SetCursorByte / SetSelCursorByte / SetSelAnchorByte
  │    └─ SetPreedit(text, cursor)
  │
  └─ plugin.render()
       └─► PreviewEngine::render
             ├─ needs_rebuild? → rebuild_layout （SourceChanged / StyleChanged / ViewportChanged）
             │     └─ LazyLayout::reserve_extra_blank_source_lines  ← 空源码行"预留高度"的入口
             ├─ EngineDirty::CursorMoved → 精细失效受影响 flat_lines
             └─ 命中 cached_dl 时快速路径
```

光标绘制路径（一切最终依赖的三个函数）：

```
cursor_screen_pos_for_byte(cursor_byte)
  │
  ├─► empty_source_line_cursor_screen_pos       ← 空源码行分支（view.rs:1066）
  │     └─► empty_source_line_metrics           ← 两套公式：between + neighbor-only（view.rs:1091）
  │
  ├─► visual_cursor_screen_pos(preedit)         ← IME preedit 分支（view.rs:1009）
  │
  └─► 常规 flat_line 分支
        └─ find_flat_and_grapheme_for_byte + grapheme_x + trailing_stripped_space_advance
```

## 二、根因归纳（"反复补丁"背后的三条主线）

看历史提交 `48d58a87 → 6954d704 → f8038bc8 → 3ffac93e → 69864f83 → 3ffac93e → 2e93b4fe → a16f642d → bdcd1d43 → 8a87247a` 等一连串修复，全部集中在同一个坐标系不自洽 + 分类不齐全 + 刷新时机粗糙的问题。

### 根因 A：源码坐标（source_line）和视觉坐标（flat_line）**两套系统各自计算**，缺一个明确的桥

- `empty_source_line_metrics`：只根据"前后最近的 flat_line + newline 个数"折算高度，与 `layout::reserve_extra_blank_source_lines` 是**另一套公式**（一个按 `line_height * N`，一个按 `paragraph_spacing + (N-2)*line_height`）。这是 `plans_wysiwyg_enter_fix.md` 描述的 Bug 1 的根源。
- `empty_source_line_role`：为了绕开 A 的自洽问题，又新造了一个"HiddenBlockSeparator"角色（`view.rs:1693`），让某些空源码行不产生光标位置。这实际上是**把不自洽掩盖成一个规则表**，规则表的边界与 `reserve_extra_blank_source_lines` 的门（`empty_line_count.saturating_sub(1)`）**不完全同步**——因此仍出现"光标画在空隙里、输入后位置又跳一下"。
- `visual_move` 左右键走 `source_line_at_byte` 判断边界；上下键走 `flat_line_source_maps`。**同一次导航切换坐标系**：在段末按 → 是 flat_line 内的 grapheme+1；在段末+1（空行）按 → 是 source_line 层的 previous_non_empty。转换点没有测试覆盖 CJK/wrap 组合。

### 根因 B：`EnterContext` 分类器**语义不完整**，分类器和增强执行**紧耦合**

- `classify_enter_context` 一趟 pulldown-cmark 事件遍历中同时判断 heading / paragraph / list / blockquote / codeblock / table / empty，逻辑分散在 15+ 处 `if let Some((start, ..)) = ...take() && current_byte >= start && current_byte <= range.end`。任何新语义（如"段末紧邻下一块起始"、"标题内 vs 标题末"）都要重新回来加一个 if。已经出现的漏洞：
  - `ParagraphInterior` 与 `EmptyBlockSeparatorLine` 的先后判定曾经错过（`plans_wysiwyg_enter_fix.md` §5.5 场景）
  - Heading 中间按 Enter 曾直接切成"半标题 + 半段落"（`3ffac93e` 才修）
  - Blockquote 段中按 Enter 曾插单 `\n` 丢引用（本方案第 §5.2 之前）
- 增强本身分散在 `paragraph_enter_augmentation` / `heading_enter_augmentation` / `list_item_enter_augmentation` / `blockquote_enter_augmentation` / `source_newline_augmentation` / `paragraph_break_augmentation` / `paragraph_break_before_existing_newline_augmentation` 共 7 个自由函数，各自决定 `insert_text`, `replace_range`, `cursor_byte_after`，**没有共享不变量的地方**：
  - `TopLevelParagraphEnd` 用 `paragraph_break_augmentation("\n\n")`
  - `ParagraphInterior` 也走同一个（复用）—— **本质上是两种语义合并成一个函数**
  - `EmptyBlockSeparatorLine` 用 `source_newline_augmentation("\n")`
  - Heading 内又区分 `bytes[current]==b'\n'` / `cursor_touches_source_newline` / else 三条支路
  分支多、共用少、缺内部一致性断言。

### 根因 C：光标状态**三处冗余**，输入后的同步靠约定而非不变量

同一时刻，"光标"的存在形态：
1. `DocumentView::cursor_offset()`（内部的 gap buffer + logical cursor，唯一被 `execute_edit_command_v2` 使用）
2. `PreviewEngine::edit_ctx.cursor_byte`（`view.rs:231`，插件侧渲染依赖）
3. 插件侧 `sel_cursor_byte` / `sel_anchor_byte`（选择、拖选、双击等 UI 事件依赖）

`dispatch_wysiwyg_augmented_edit` 的落地顺序是：
1. `position_document_for_wysiwyg_replace_range` → 只改 `dv.cursor` + `selection_anchor`
2. `execute_augmentation_text_change` → 走 `execute_edit_command_v2`，改变 gap buffer + generation
3. 若 `cursor_offset != cursor_byte_after` → `dv.cursor_move_to_offset(cursor_byte_after)`
4. `sync_plugin_state()` → 单向推 `SetCursorByte(dv.cursor_offset())` 到插件

问题：
- 步骤 1 后、步骤 2 前，插件端的 `edit_ctx.cursor_byte` 依然是**旧值**。此时如果 `execute_edit_command_v2` 内部触发了任何 query（比如未来加了 hook），就会看到不一致的中间态。
- 步骤 4 是唯一 push 到插件的地方，但 `sync_plugin_state` 内部只在 `NeedsSourceUpdate == true` 时才 rebuild 源码字符串，otherwise 光标 push 优先——**首次输入后 generation 立刻涨，source 又要 rebuild 一整份 String**（长文档下不便宜）。
- `set_preedit_text` 在 IME 期间会自己改 `edit_ctx.cursor_byte`（`view.rs:302`），如果这时候用户又按方向键，`dispatch_wysiwyg_navigation` 会用**插件里那个 preedit 阶段被临时改过的 cursor_byte** 去查 `VisualMove`，从而出现 issues.md 里的 "IME preedit 时光标在最左侧" 现象。

### 根因 D：刷新触发**过于宽泛**

- `handle_cursor_moved` 每一次鼠标移动（>60Hz）都会：
  - `dispatch_mouse` 做一遍完整的 overlay/dock hit-test
  - 若 overlays 存在（哪怕鼠标其实没落到任何 hover 目标上）→ `actions.push(AppAction::RequestRedraw)`（events.rs:266）
- 这直接对应 issues.md 的"鼠标没动,但拼命刷新"（overlays 常驻时的情况）以及 "hover 刷新慢"。
- 现有 dirty 分类只有 `SourceChanged / StyleChanged / ViewportChanged / CursorMoved / Clean` 5 档，`sync_plugin_state` 又是每帧都跑，即使 `Clean` 也要走完 needs_update 判断 + selection 对齐 + preedit push。

### 根因 E：`reserve_extra_blank_source_lines` 与 `empty_source_line_metrics` 各自算高度

- Layout 侧：`reserve_extra_blank_source_lines` 把"块间的多余空源码行"按 `line_height * (empty - 1)` 直接加到 `y_delta` 上（`types.rs:889`）。第一条空行不占空间——被视为 block separator。
- 视觉侧：`empty_source_line_metrics_between` 反过来根据 `prev.rect + gap_height - editable * (empty-1)` 反推 separator 高度（`view.rs:1180`）。
- 两侧必须严格互逆才不出现"光标画在空隙里没有 flat_line 承载"或"点空源码行选不中"。目前用 `EmptySourceLineRole` 兜住不一致时的兜底显示，但边界（例如首行空、末行空、连续 3+ 空行、光标恰好在 run 中）就会时不时露馅。

---

## 三、具体问题清单（issues.md → 根因映射）

| issues.md 现象 | 直接触发点 | 根因 |
|---|---|---|
| YAML 区域折行不对 | wrap 算法对代码块前置未识别 YAML 前后 fence | 与本主题弱相关，另开单 |
| 鼠标没动却拼命刷新 | `handle_cursor_moved` overlays 分支强制 RequestRedraw | D |
| 左右箭头无效 | `wysiwyg_route_for_command → Navigation → visual_move Left/Right`；在空行边界回 `Some(current_byte)` 而不是 previous.end | A + B |
| IME preedit 光标在最左侧 | `set_preedit_text` 改 `edit_ctx.cursor_byte`，之后 `visual_cursor_screen_pos` 拿到的 flat_line 匹配失败落到空行分支 | A + C |
| 空格状态不对 | `augment_insert_text` 只处理 `EmptyBlockSeparatorLine`；空格在段末走 fallback，光标位置不同步 | B + C |
| 回车卡顿 | 每次 Enter 后 `sync_plugin_state` rebuild 整个 String + engine `SourceChanged` 全量 rebuild_layout | C + E |
| 段末回车出现"很大空行"（已修） | `empty_source_line_metrics` 换算 vs `reserve_extra_blank_source_lines` | A + E |
| 段间空行回车"只加空隙"（已修） | 分类器缺 `EmptyBlockSeparatorLine` | B |
| 标题后回车（已修） | Heading `at_end` 三条支路，其中一支曾错 | B |
| 段落间空白不可点击 | `hit_test_byte` 里的 EmptySourceLineRole hidden 兜底 | E |
| 方向键上下不对（已修） | `visual_move` 从空源码行的 vertical 分支曾 fall through | A |

---

## 四、根治方案（分层）

方案分四层，从底往上做，每层可独立验证：

### L1. 源码 ↔ 视觉的**桥接层**：`SourceLineMap`（消除根因 A / E）

**目标**：把"源码某字节对应到哪一行；那一行是否为空；那一行的视觉 y/height；那一行是 hidden separator 还是 editable"这类查询集中到一个数据结构。

**新增**（放在 `crates/markdown/src/layout/source_line_map.rs`）：

```rust
pub struct SourceLineMap<'a> {
    source: &'a str,
    // 单趟扫描得到的每一行：起止字节 + 是否空 + 关联的 flat_line 范围 + 归属角色
    lines: Vec<SourceLineEntry>,
    // 反查：source_byte → line_index（二分或直接 prefix 表）
    line_starts: Vec<usize>,
}

pub struct SourceLineEntry {
    pub index: usize,
    pub start: usize,
    pub end: usize,          // == start 时表示空行
    pub flat_range: Range<usize>,  // 该源码行渲染成的 flat_lines 区间；空行时 range 为空
    pub role: SourceLineRole,      // Rendered | HiddenBlockSeparator | EditableEmpty
    pub y_top: f32,          // 绝对 y（文档坐标）
    pub height: f32,         // 该行占据的高度（对空行来说是 paragraph_spacing 或 line_height）
}

pub enum SourceLineRole {
    Rendered,             // 非空行，被 flat_line 承载
    HiddenBlockSeparator, // 空行，作为块分隔，占 paragraph_spacing
    EditableEmpty,        // 空行，可被光标停留，占 line_height
}
```

**在 `PreviewEngine::rebuild_layout` 结束前构建一次**，替换掉：
- `source_line_at_byte`
- `source_line_by_index`
- `empty_source_lines`
- `empty_source_line_rank`
- `empty_source_line_role`
- `surrounding_rendered_lines`
- `empty_source_line_metrics` / `..._between`

以后所有"光标位置 / hit-test / visual_move / augment"读同一份 SourceLineMap。**不再需要 `reserve_extra_blank_source_lines` 独立算一遍高度**——直接让 `SourceLineMap` 承担 y_delta 的累计输出，然后一次性写回 `LazyLayout::y_delta`。

**验收**：
- `empty_source_line_metrics` 全部换成 `source_line_map.entry(line).height`
- 删除 `EmptySourceLineRole` 枚举（并进 `SourceLineRole`）
- 单元测试：随机 fuzz 段落 / 空行组合，断言 `sum(line.height) + first_flat.y == last_flat.y + last_flat.h + trailing`

### L2. Augmenter 拆分：分类器 vs 执行器（消除根因 B）

**目标**：`classify_enter_context` 只输出结构化上下文；execution 由一个纯函数表 `Augmenter` 完成，语义共享同一份不变量。

**改动**：

```rust
// crates/markdown/src/edit/context.rs
#[derive(Debug)]
pub enum EditContextKind {
    ParagraphEnd,
    ParagraphInterior,
    ParagraphStart,           // 新：段首（如首字前）
    HeadingEnd { level: u8 },
    HeadingInterior { level: u8 },
    ListItemContentEnd { indent, bullet },
    ListItemContentInterior { indent, bullet },
    ListItemEmpty { indent, bullet },
    BlockQuoteLineEnd,
    BlockQuoteLineInterior,
    BlockQuoteLineEmpty,
    CodeBlock,
    TableCell { next_cell_start, prev_cell_start },
    EmptyBlockSeparator { has_prev_block: bool, has_next_block: bool },
    Other,
}

pub struct EditContextClassifier<'a> {
    source: &'a str,
    source_map: &'a SourceLineMap<'a>,
}

impl EditContextClassifier<'_> {
    pub fn classify(&self, byte: usize) -> EditContextKind { .. }
}
```

分类器读 `SourceLineMap` 和 pulldown-cmark 事件，**决策放在同一函数里**（不再散在 15 处 take-if-let）。用严格的 match 覆盖所有 `EditContextKind`，编译器兜底穷尽性。

Augmenter 拆分成 3 张纯函数表：

```rust
pub trait EditAugmenter {
    fn augment_enter(&self, ctx: EditContextKind, byte: usize) -> Option<EditAugmentation>;
    fn augment_backspace(&self, ctx: EditContextKind, byte: usize) -> Option<EditAugmentation>;
    fn augment_insert_text(&self, ctx: EditContextKind, byte: usize, text: &str) -> Option<EditAugmentation>;
}
```

- 命名：把 `paragraph_break_augmentation` / `source_newline_augmentation` 之类的辅助合并成 3 个：
  - `emit_source_newline(byte)` — 单个 `\n`
  - `emit_block_break(byte)` — `\n\n`，跨越 block
  - `emit_marker_break(byte, indent, marker)` — list/bq 续 marker
- 内部**共享不变量**：
  - `cursor_byte_after` 必须 = `range.start + insert_text.len() - trailing_newlines_kept`
  - `replace_range` 若非空，则 `insert_text.starts_with(prefix_of_source_at_range.start)` 断言
  - 用 debug_assert 强制

**验收**：
- 每一种 `EditContextKind` 至少 3 个测试（Enter / Backspace / InsertText 各一个）
- `augment_edit` 主入口≤ 30 行；决策全部落在 `Augmenter::augment_*` 内
- 删除 `paragraph_break_before_existing_newline_augmentation`（合并到 `emit_block_break` + edge 检测）

### L3. 光标状态**唯一真相源**（消除根因 C）

**目标**：`DocumentView::cursor_offset()` 是唯一权威；插件侧的 `edit_ctx.cursor_byte` 是**镜像**，永远由 push 更新，绝不主动写。

**规则**（写在 `docs/plans/cursor-source-of-truth.md` 常驻）：
1. **写方向单向**：任何修改光标的路径必须调用 `set_wysiwyg_cursor_and_selection`（现已存在，`wysiwyg.rs:248`）。禁止再有第二个地方直接 `dv.cursor_move_to_offset()` + 单独 push。
2. **preedit 不能改 `edit_ctx.cursor_byte`**：`set_preedit_text` 只写 `preedit_text` / `preedit_cursor`；`visual_cursor_screen_pos` 用 `cursor_byte + preedit_cursor_byte_offset` 派生显示位置。已经有 `visual_cursor_screen_pos`（`view.rs:1009`），只需**去掉 `set_preedit_text` 里覆写 `cursor_byte` 的路径**（`view.rs:302-313`）。
3. **augment 的原子性**：`dispatch_wysiwyg_augmented_edit` 收敛成一个 op：
   ```
   op = {
     delete: replace_range,      // 可为空
     insert: insert_text,        // 可为空
     cursor_after: byte,
     selection_after: Option<Range>,
   }
   ```
   一次性走 `execute_edit_command_v2(EditCommand::ReplaceRange { range, text })`（新增指令，或者复用 `DeleteRange + InsertText` 但保证在同一次 outcome 里）。中间不允许 sync。**结束后一次 push**。
4. **sync_plugin_state 的两个模式**：
   - `full_sync`：当 `SourceChanged` 或 `plugin.needs_update()` 为真——rebuild source 字符串 + 双向对齐 selection + push cursor
   - `light_sync`：其他情况——只 push cursor / preedit / selection 变化（用 `PluginQuery::CurrentCursorByte` 比对，不同则 push）
   render() 默认走 light_sync；`execute_edit_command_v2` 出口显式请求 full_sync。

**验收**：
- 单元测试：`set_preedit_text` 前后 `edit_ctx.cursor_byte` 保持不变
- IME 期间按方向键：`dispatch_wysiwyg_navigation` 读到的 `current_byte == dv.cursor_offset()`
- benchmark：大文档（10k 行）连续 100 次 InsertChar 的 sync 平均 <100µs（对比现在 rebuild 整个 String 的耗时）

### L4. 刷新门控（消除根因 D）

**目标**：mouse move 只在真正改变了 hover 目标或 cursor icon 时才请求重绘。

**改动点**：

- `handle_cursor_moved`（`events.rs:265`）：
  ```rust
  if app.ui_shell.overlays_count() > 0 {
      actions.push(AppAction::RequestRedraw);
  }
  ```
  改为：
  ```rust
  if app.ui_shell.overlay_hover_changed(px, py) {
      actions.push(AppAction::RequestRedraw);
  }
  ```
  在 `ui_shell` 上新增 `overlay_hover_changed(x, y) -> bool`，内部记忆上一次命中的 overlay id + hover state。**只在跨越 overlay 边界或 hover state 翻转时返回 true**。

- Tab bar hover：`AppAction::HoverTab(None)` 在鼠标离开 tab bar 区域时反复推送但值都是 None → 加 dedupe：只有当上一次是 `Some(_)` 时才推。

- `EngineDirty::CursorMoved` 分支：目前 `set_edit_ctx(self.edit_ctx.clone())` 后跑 `lazy.invalidate_lines_for_source_bytes(...)` 全量 `ensure_all_blocks`（`view.rs:571-580`）。当 `full_layout=true`（编辑模式）时这条路径每次光标移动都跑 all blocks——即使实际只有两行需要反染色。**改为**：只 invalidate 涉及的两个 flat_line（old + new），其余保留缓存。

**验收**：
- 手动：鼠标在空白区域移动，`RequestRedraw` frame 计数应为 0
- 手动：TOC 上悬停，进入/离开单元格时才刷新
- benchmark：InsertChar 后再 render() 的 `perf_cursor_us` < 500µs

---

## 五、实施顺序与拆解（阶段 = commit 边界）

严格按依赖顺序做，每阶段独立编译 + 覆盖测试 + `./scripts/verify.sh`。

### 阶段 0：诊断与测试红灯（1 commit）

写下面失败测试：

1. `empty_source_line_role_never_overlaps_flat_lines`：随机 source 生成，断言 `role==HiddenBlockSeparator` 的行 y_top+height == 相邻 flat_line 的 y（互逆性）
2. `set_preedit_does_not_move_edit_ctx_cursor`：设置 cursor_byte=5，push preedit → cursor_byte 仍为 5
3. `augmented_insert_text_produces_single_outcome`：InsertText 增强后 outcome 里 `new_line_count - old_line_count` 与 augmentation 的 newline 数匹配
4. `mouse_move_over_empty_area_does_not_request_redraw`：合成一次 hover 无 overlay 命中的事件，assert `RequestRedraw` 计数 = 0
5. `arrow_left_from_empty_line_middle_returns_prev_line_end`：`para1\n\npara2` cursor 6，MoveLeft → 5（不是 6）
6. `arrow_during_ime_uses_doc_cursor_not_preedit_cursor`：cursor=5，preedit="ab"，push preedit，MoveRight 得 6

### 阶段 1：L1 桥接层（3-4 commit）

- 1a：新增 `SourceLineMap` 数据结构（不接入）
- 1b：`empty_source_line_metrics` 全部走 SourceLineMap，删除 `EmptySourceLineRole`
- 1c：`reserve_extra_blank_source_lines` 的高度累加转由 SourceLineMap 输出，`y_delta` 一次性写回
- 1d：清理 `empty_source_lines` / `empty_source_line_rank` / `surrounding_rendered_lines` 的自由函数（合入 SourceLineMap 方法）

### 阶段 2：L2 Augmenter 拆分（3 commit）

- 2a：新增 `EditContextKind` 枚举 + `EditContextClassifier`（返回一致结果，先与 `classify_enter_context` 并存，跑 diff 测试）
- 2b：新增 `EditAugmenter` trait，把 7 个自由函数搬进来（先只处理 Enter），删掉旧的 `enter_context_augmentation`
- 2c：Backspace + InsertText 迁到 `EditAugmenter`；删除 `augment_insert_text` 自由函数

### 阶段 3：L3 光标唯一真相源（2 commit）

- 3a：`set_preedit_text` 不再改 `edit_ctx.cursor_byte`，`visual_cursor_screen_pos` 用 preedit_cursor 偏移派生。同时给 `dispatch_wysiwyg_navigation` 加断言：`current_byte == dv.cursor_offset()`
- 3b：新增 `EditCommand::ReplaceRange { range, text }`（复用 `execute_edit_command_v2` 的 selection 展开路径），`dispatch_wysiwyg_augmented_edit` 收敛到单一命令；`sync_plugin_state` 拆 `full_sync` / `light_sync`

### 阶段 4：L4 刷新门控（2 commit）

- 4a：`ui_shell` 增加 `overlay_hover_changed`；`handle_cursor_moved` overlay 分支切换；`HoverTab(None)` dedupe
- 4b：`EngineDirty::CursorMoved` 走精细失效（只 invalidate 老/新 flat_line），压 benchmark

### 阶段 5：清理（1 commit）

- 删除已被替代的辅助函数、常量、注释
- 更新 `docs/plans/2026-07-01-wysiwyg-enter-behavior-fix.md` 和 `docs/plans_wysiwyg_enter_fix.md` 顶部加"已被 2026-07-06 方案取代"

---

## 六、已确定的三处决策（2026-07-06 用户拍板）

1. **段中间按 Enter 的语义**：按 **Typora**——把段切两半，`\n\n`，光标落在第二段首字。保留现状 `paragraph_break_augmentation("\n\n")`。
2. **块间空行按 Enter 的语义**（`EmptyBlockSeparatorLine`）：**再加一个空行**（在当前空行处插入 `\n`）。保留现状 `source_newline_augmentation("\n")`。
3. **`EditCommand::ReplaceRange { range, text }`**：**新增**。要求 augment 场景一次性 outcome + cursor 原子落位；老的 `DeleteRange + InsertText` 组合仅作为其内部实现细节。

---

## 七、风险与回退

- **L1 桥接层重构风险最高**：改动 `layout` 与 `view` 的合作面。回退策略：桥接层单独一个 crate 子模块，接入前先跑一个 `assert_eq!(old_metrics, new_metrics)` 的 shadow 对比 pass，一周不出问题再删除老路径。
- **L3 preedit 修改可能引发 IME 表现回归**：macOS / Linux fcitx / Windows 表现差异较大。上线前手工测三家 IME + 硬编码单元测试覆盖 preedit 空/单字/多字/删除的 4 组合。
- **L4 刷新收敛可能漏刷 hover**：所有依赖 hover 状态的组件需要显式 opt-in（在 `overlay_hover_changed` 里返回 true）。上线前遍历 sidebar / tab_bar / TOC / search_bar / scrollbar 五个组件。

---

## 八、成功指标

| 指标 | 现状 | 目标 |
|---|---|---|
| WYSIWYG 空行 / 光标相关 issues 数（issues.md） | 8 | 0 |
| `classify_enter_context` 圈复杂度 | 25+ | ≤ 12 |
| `augment_*` 自由函数数量 | 7 | 3 |
| 大文档（10k 行）InsertChar → 下一帧 render 时间 | ≈ 2ms（含 rebuild source） | ≤ 500µs |
| 无 hover 目标时鼠标移动 RequestRedraw 频率 | 每次 mouse move | 0 |
| WYSIWYG 相关单元测试覆盖 | ≈ 40 | ≥ 80 |

---

## 九、附：文件改动总览（预估）

| 文件 | 阶段 | 改动性质 |
|---|---|---|
| `crates/markdown/src/layout/source_line_map.rs` | L1 | 新增 |
| `crates/markdown/src/layout/types.rs` | L1 | 删除 `reserve_extra_blank_source_lines` 独立逻辑 |
| `crates/markdown/src/view.rs` | L1 / L2 / L3 | 删 ~400 行、加 ~200 行 |
| `crates/markdown/src/edit/context.rs` | L2 | 新增 |
| `crates/markdown/src/edit/augmenter.rs` | L2 | 新增 |
| `crates/app/src/dispatch/wysiwyg.rs` | L3 | `dispatch_wysiwyg_augmented_edit` 精简 |
| `crates/app/src/app_renderer.rs` | L3 | `sync_plugin_state` 拆 full / light |
| `crates/app/src/events.rs` | L4 | hover redraw 门控 |
| `crates/ui/src/plugin.rs` | L3 | 可能新增 `EditCommand::ReplaceRange`（若接受方案六.3） |
| `crates/ui/src/ui_shell/` | L4 | `overlay_hover_changed` 接口 |

结束。
