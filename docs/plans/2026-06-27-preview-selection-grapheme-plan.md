# Preview Selection Grapheme Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Markdown preview-only 模式的文本选择、复制、鼠标拖拽、高亮、双击选词和键盘扩展选择全部按 UAX #29 grapheme 边界工作，而不是按 Rust `char` 位置工作。

**Architecture:** 保持现有 plugin 自渲染架构：`app` 仍通过 `PluginQuery` / `PluginMessage` 驱动 preview selection；`ui` 只扩展纯数据协议；`markdown` plugin 内部把 `ViewPos` 第二维从 char index 迁移为 grapheme index，并在选中文本切片和高亮 x 坐标计算时统一使用 grapheme 边界。该计划只覆盖 preview-only selection，不处理 WYSIWYG source byte cursor；WYSIWYG 编辑态见 `docs/plans/2026-06-27-mdeditor-grapheme-cursor-state-plan.md`。

**Tech Stack:** Rust workspace；`edit-plus-ui` 提供纯 plugin 协议；`edit-plus-app` 负责 preview 模式输入调度；`edit-plus-markdown` 负责 preview layout、hit-test、selection、copy/highlight；验证使用 `cargo test`、`cargo check`、`cargo fmt`，重大修改后执行 `./scripts/verify.sh`。

## Global Constraints

- 全程遵守 `AGENTS.md`：中文沟通，行动前说清方案，遇 bug 先写复现测试再修。
- 单任务修改超过 3 个文件必须拆分为子任务；每次提交前必须确保编译通过。
- `crates/ui` 只能定义纯数据协议，绝对禁止依赖或访问 `crates/app` 状态结构体。
- 新增跨层数据必须是纯数据 struct 或 enum variant，不得把 Markdown 内部 `ViewPos` 泄漏给 `ui`。
- Preview selection 必须从 `Position(Option<(usize, usize)>)` 拆出强类型协议；禁止继续用 tuple 表示文本选择坐标。
- 所有 selection copy/highlight/word boundary/line boundary 都必须在 UTF-8 char boundary 和 UAX #29 grapheme boundary 上切分。
- 命名必须精准自解释，禁止新增 `data` / `info` / `temp` / `res` / `flag` 等宽泛名称。

---

## 当前状态全景

### 1. App preview selection 状态

| 状态 / 路径 | 位置 | 当前职责 | 问题 |
|-------------|------|----------|------|
| Preview click | `crates/app/src/dispatch/mouse.rs` | `HitTest -> Position((li, cp))` 后写 `SetSelAnchor/SetSelCursor` | `cp` 是 char index |
| Preview drag | `crates/app/src/dispatch/mouse.rs` | 鼠标按下移动时持续 `SetSelCursor((li, cp))` | 拖过 combining/ZWJ 时可能停在 grapheme 内部 |
| Double-click word | `crates/app/src/dispatch/mouse.rs` | 查询 `WordAtPos(li, cp)` 后设置 selection | word 边界基于 `Vec<char>` |
| Triple-click line | `crates/app/src/dispatch/mouse.rs` | 查询 `LineRangeAtPos(li, cp)` | 行尾位置用 `text.chars().count()` |
| Keyboard ExtendLeft/Right | `crates/app/src/dispatch/editor.rs` | 读取 `SelCursor` 和 `FlatLines`，用 `chars().count()` 算行长 | app 侧直接持有 char 语义 |
| Copy/Cut | `crates/app/src/dispatch/editor.rs` | 查询 `SelectedText` 写剪贴板 | 文本切片由 markdown selection 决定 |

### 2. UI plugin 协议状态

| 协议 | 当前注释 | 问题 |
|------|----------|------|
| `SetSelCursor(Option<(usize, usize)>)` | `(flat_line_idx, char_pos)` | 注释和语义都是 char；preview selection 必须迁出 |
| `SetSelAnchor(Option<(usize, usize)>)` | `(flat_line_idx, char_pos)` | 同上 |
| `PluginQuery::SelCursor` | `(line, column)` | `column` 不精确，实际是 preview line-local char index；preview selection 必须迁出 |
| `PluginQuery::HitTest` | returns `Position` | 返回 `(flat_line_idx, char_pos)`；preview text hit-test 必须迁出 |
| `PluginQuery::WordAtPos` | `(flat_line_idx, char_pos)` | 输入输出都是 char；preview word range 必须迁出 |
| `PluginQuery::LineRangeAtPos` | `(flat_line_idx, char_pos)` | 输入输出都是 char；preview line range 必须迁出 |
| `PluginResponse::FlatLines(Vec<FlatLine>)` | `FlatLine { text }` | app 只能自己 `chars().count()`，没有 grapheme_count |

### 3. Markdown preview selection 状态

| 状态 | 位置 | 当前职责 | 问题 |
|------|------|----------|------|
| `ViewPos { flat_line_idx, char_pos }` | `crates/markdown/src/selection.rs` | preview selection 坐标 | 第二维是 char index |
| `hit_test()` | `selection.rs` | screen point -> `ViewPos` | 调用 `char_at_x()` |
| `word_at_pos()` | `selection.rs` | 双击选词 | `Vec<char>` 分类，combining mark 会独立成标点/非 word |
| `line_range_at_pos()` | `selection.rs` | 三击选行 | 行尾是 `text.chars().count()` |
| `SelectionState::selected_text()` | `selection.rs` | copy selected text | 用 `text.char_indices().nth()` 切片 |
| `SelectionState::highlights()` | `selection.rs` | selection rectangle | 调用 `char_x()` |
| `SearchState` | `crates/markdown/src/search.rs` | search highlight | 也用 char index；本计划不迁移 search，search 可作为后续计划 |

### 4. Layout x 坐标状态

| 函数 | 位置 | 当前职责 | 问题 |
|------|------|----------|------|
| `char_at_x()` | `crates/markdown/src/layout/context.rs` | x -> char index | shaped cluster 中通过 `text[..cluster.byte_range.start].chars().count()` 回 char |
| `char_x()` | `crates/markdown/src/layout/context.rs` | char index -> x | 用 `text.char_indices().nth(visual_char)` 找 byte |
| `FlatLine::shaped` | `crates/markdown/src/layout/types.rs` | 实际 glyph cluster | 选择坐标与渲染 cluster 之间缺少 grapheme 中间层 |

## 根因结论

Preview-only selection 当前表面上是 `(line, column)`，实际是 `(flat_line_idx, char_pos)`。这在 ASCII 和大多数 CJK 文本上看起来正常，但在以下文本中会失真：

- NFD：`e\u{0301}` 会被当成两个 char，选择可能只复制 combining mark 或只高亮 base。
- ZWJ emoji：`👨‍👩‍👧` 会被拆成多个 char，键盘扩展选择和拖拽可以停在 emoji 内部。
- Variation selector：`✈️` 可能拆成 base + VS16，highlight 宽度和复制范围不一致。
- 双击选词：combining mark 的 char class 不等于 base letter，`word_at_pos()` 会把一个用户感知字符拆开。

目标是把 preview selection 从通用 `Position(Option<(usize, usize)>)` 协议中拆出来，用 `PreviewTextPosition` / `PreviewTextRange` 明确表达 flat line 与 visual grapheme index。`Position` 可以继续服务 TOC、mindmap、旧非文本场景，但 Markdown preview 文本选择不得再复用它。

---

## Target Model

```text
PreviewViewPos {
  flat_line_idx: usize,
  grapheme_idx: usize,
}

ui protocol:
  PreviewTextPosition { flat_line_idx, grapheme_idx }
  PreviewTextRange { start: PreviewTextPosition, end: PreviewTextPosition }

FlatLine:
  text: String
  grapheme_count: usize
```

所有 preview selection 操作按下面链路运行：

```text
screen x
  -> visual grapheme index
  -> ViewPos(flat_line_idx, grapheme_idx)
  -> grapheme-safe selected_text slice
  -> grapheme-safe highlight x range
```

## File Structure

- Create: `crates/markdown/src/grapheme.rs`
  - Markdown preview 内部 grapheme 边界工具：计数、byte lookup、x hit-test 辅助。
- Modify: `crates/markdown/src/lib.rs`
  - 注册 `grapheme` 模块。
- Modify: `crates/markdown/src/layout/context.rs`
  - 新增 `grapheme_at_x()` / `grapheme_x()`；保留 `char_at_x()` / `char_x()` 给 search 或旧路径。
- Modify: `crates/markdown/src/selection.rs`
  - `ViewPos.char_pos` 改为 `grapheme_idx`；copy/highlight/word/line 全部迁移。
- Modify: `crates/markdown/src/view.rs`
  - `query_common()` 和 `MarkdownView` / `NovelView` 改用 preview selection 强类型消息/查询；`FlatLines` 返回 grapheme_count。
- Modify: `crates/ui/src/plugin.rs`
  - 新增 `PreviewTextPosition` / `PreviewTextRange`；新增 preview selection 专用 message/query/response；`FlatLine` 新增 `grapheme_count`。
- Modify: `crates/app/src/dispatch/editor.rs`
  - preview keyboard extend 使用 `FlatLine.grapheme_count`。
- Modify: `crates/app/src/dispatch/mouse.rs`
  - preview mouse selection 改用 `PreviewHitTest` / `SetPreviewSelection*` 强类型协议。

---

## Task 1: 新增 Markdown Preview Grapheme 工具

**Files:**
- Create: `crates/markdown/src/grapheme.rs`
- Modify: `crates/markdown/src/lib.rs`
- Test: `crates/markdown/src/grapheme.rs`

**Interfaces:**
- Produces: `pub(crate) fn grapheme_count(text: &str) -> usize`
- Produces: `pub(crate) fn byte_index_for_grapheme(text: &str, grapheme_idx: usize) -> usize`
- Produces: `pub(crate) fn grapheme_index_for_byte(text: &str, byte_idx: usize) -> usize`
- Produces: `pub(crate) fn grapheme_slices(text: &str) -> Vec<std::ops::Range<usize>>`

- [ ] **Step 1: Write failing tests**

Add tests:

```rust
#[test]
fn grapheme_count_handles_combining_mark() {
    assert_eq!(grapheme_count("xe\u{0301}y"), 3);
}

#[test]
fn byte_index_for_grapheme_skips_combining_mark() {
    let text = "xe\u{0301}y";
    assert_eq!(byte_index_for_grapheme(text, 0), 0);
    assert_eq!(byte_index_for_grapheme(text, 1), 1);
    assert_eq!(byte_index_for_grapheme(text, 2), 1 + "e\u{0301}".len());
    assert_eq!(byte_index_for_grapheme(text, 3), text.len());
}

#[test]
fn grapheme_count_handles_zwj_emoji() {
    let emoji = "👨\u{200D}👩\u{200D}👧";
    assert_eq!(grapheme_count(&format!("x{emoji}y")), 3);
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test -p edit-plus-markdown grapheme_count_handles --lib`

Expected: FAIL because `crates/markdown/src/grapheme.rs` does not exist.

- [ ] **Step 3: Implement with existing UCD grapheme helpers**

Use:

```rust
use core::unicode::{
    ucd_grapheme_cluster_joins,
    ucd_grapheme_cluster_joins_done,
    ucd_grapheme_cluster_lookup,
};
```

Implementation rules:

- First char starts a grapheme.
- A new grapheme starts when `ucd_grapheme_cluster_joins_done(state)` returns true.
- `byte_index_for_grapheme(text, grapheme_count(text))` returns `text.len()`.
- Input index larger than count clamps to `text.len()`.

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p edit-plus-markdown grapheme_count_handles --lib
cargo test -p edit-plus-markdown byte_index_for_grapheme --lib
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/grapheme.rs crates/markdown/src/lib.rs
git commit -m "test: add markdown preview grapheme helpers"
```

## Task 2: 新增 Preview Selection 强类型协议

**Files:**
- Modify: `crates/ui/src/plugin.rs`
- Modify: `crates/markdown/src/view.rs`
- Test: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: `grapheme_count(text: &str)` from Task 1
- Produces: `ui::plugin::FlatLine { text: String, grapheme_count: usize }`
- Produces: `ui::plugin::PreviewTextPosition { flat_line_idx: usize, grapheme_idx: usize }`
- Produces: `ui::plugin::PreviewTextRange { start: PreviewTextPosition, end: PreviewTextPosition }`
- Produces: preview-only message/query/response variants that do not reuse `Position((usize, usize))`

- [ ] **Step 1: Write failing strong-protocol tests**

In Markdown preview tests, assert NFD line count and typed hit-test:

```rust
#[test]
fn preview_protocol_reports_grapheme_count_and_typed_position() {
    use ui::plugin::{PluginQuery, PluginResponse, ViewPlugin};

    let doc = StubDoc::new("xe\u{0301}y");
    let mut view = MarkdownView::new("xe\u{0301}y".to_string(), 1);
    render_preview_once(&mut view, &doc);

    let lines = match view.query(PluginQuery::FlatLines, &doc) {
        PluginResponse::FlatLines(lines) => lines,
        other => panic!("expected FlatLines, got {other:?}"),
    };

    assert_eq!(lines[0].text, "xe\u{0301}y");
    assert_eq!(lines[0].grapheme_count, 3);

    let hit = match view.query(
        PluginQuery::PreviewHitTest { x: 0.0, y: 0.0, offset_x: 0.0, offset_y: 0.0 },
        &doc,
    ) {
        PluginResponse::PreviewTextPosition(position) => position,
        other => panic!("expected PreviewTextPosition, got {other:?}"),
    };
    assert!(hit.is_some());
}
```

- [ ] **Step 2: Run test to verify failure**

Run: `cargo test -p edit-plus-markdown preview_protocol_reports_grapheme_count_and_typed_position --lib`

Expected: FAIL because `FlatLine` has no `grapheme_count` field and `PreviewHitTest` / `PreviewTextPosition` variants do not exist.

- [ ] **Step 3: Add UI protocol types**

Add pure data structs to `crates/ui/src/plugin.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewTextPosition {
    pub flat_line_idx: usize,
    pub grapheme_idx: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewTextRange {
    pub start: PreviewTextPosition,
    pub end: PreviewTextPosition,
}
```

Add preview-only message variants:

```rust
SetPreviewSelectionCursor(Option<PreviewTextPosition>),
SetPreviewSelectionAnchor(Option<PreviewTextPosition>),
```

Add preview-only query variants:

```rust
PreviewSelectionCursor,
PreviewHitTest { x: f32, y: f32, offset_x: f32, offset_y: f32 },
PreviewWordAt(PreviewTextPosition),
PreviewLineRangeAt(PreviewTextPosition),
```

Add response variants:

```rust
PreviewTextPosition(Option<PreviewTextPosition>),
PreviewTextRange(Option<PreviewTextRange>),
```

Keep old `SetSelCursor`, `SetSelAnchor`, `SelCursor`, `HitTest`, `WordAtPos`, and `LineRangeAtPos` for non-preview-text plugin paths only. Do not use them from Markdown preview selection after this task.

- [ ] **Step 4: Add `FlatLine.grapheme_count`**

Change:

```rust
pub struct FlatLine {
    pub text: String,
}
```

to:

```rust
pub struct FlatLine {
    pub text: String,
    pub grapheme_count: usize,
}
```

- [ ] **Step 5: Update Markdown query constructors**

Every `ui::plugin::FlatLine { text: fl.text.clone() }` becomes:

```rust
ui::plugin::FlatLine {
    text: fl.text.clone(),
    grapheme_count: crate::grapheme::grapheme_count(&fl.text),
}
```

- [ ] **Step 6: Wire typed preview queries in `view.rs`**

In `PreviewEngine::query_common()` and `MarkdownView::query()`, route:

```rust
PluginQuery::PreviewSelectionCursor => {
    PluginResponse::PreviewTextPosition(
        self.sel.cursor.map(|p| PreviewTextPosition {
            flat_line_idx: p.flat_line_idx,
            grapheme_idx: p.grapheme_idx,
        })
    )
}
PluginQuery::PreviewHitTest { x, y, offset_x, offset_y } => {
    PluginResponse::PreviewTextPosition(
        self.hit_test(*x, *y, *offset_x, *offset_y).map(|p| PreviewTextPosition {
            flat_line_idx: p.flat_line_idx,
            grapheme_idx: p.grapheme_idx,
        })
    )
}
PluginQuery::PreviewWordAt(position) => {
    let (start, end) = self.word_at_pos(ViewPos {
        flat_line_idx: position.flat_line_idx,
        grapheme_idx: position.grapheme_idx,
    });
    PluginResponse::PreviewTextRange(Some(PreviewTextRange {
        start: PreviewTextPosition { flat_line_idx: start.flat_line_idx, grapheme_idx: start.grapheme_idx },
        end: PreviewTextPosition { flat_line_idx: end.flat_line_idx, grapheme_idx: end.grapheme_idx },
    }))
}
```

Use the same pattern for `PreviewLineRangeAt`.

- [ ] **Step 7: Verify**

Run:

```bash
cargo test -p edit-plus-markdown preview_protocol_reports_grapheme_count_and_typed_position --lib
cargo check -p edit-plus-app
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/ui/src/plugin.rs crates/markdown/src/view.rs
git commit -m "feat: add typed preview selection protocol"
```

## Task 3: 新增 Grapheme X 坐标转换

**Files:**
- Modify: `crates/markdown/src/layout/context.rs`
- Test: `crates/markdown/src/layout/context.rs`

**Interfaces:**
- Consumes: `byte_index_for_grapheme()` / `grapheme_index_for_byte()` from Task 1
- Produces: `pub(crate) fn grapheme_at_x(flat_line: &FlatLine, rel_x: f32) -> usize`
- Produces: `pub(crate) fn grapheme_x(flat_line: &FlatLine, grapheme_idx: usize) -> f32`

- [ ] **Step 1: Write failing x conversion tests**

Add fallback-width tests without shaper:

```rust
#[test]
fn grapheme_x_skips_combining_mark() {
    let line = FlatLine {
        flat_idx: 0,
        rect: Rect::new(0.0, 0.0, 100.0, 20.0),
        text: "xe\u{0301}y".to_string(),
        font_size: 10.0,
        shaped: None,
        source_bytes_by_visual_char: None,
    };

    let x_after_combining = grapheme_x(&line, 2);
    let x_after_base_char = char_x(&line, 2);

    assert!(x_after_combining > x_after_base_char);
    assert_eq!(grapheme_x(&line, 3), char_x(&line, line.text.chars().count()));
}
```

- [ ] **Step 2: Run test to verify failure**

Run: `cargo test -p edit-plus-markdown grapheme_x_skips_combining_mark --lib`

Expected: FAIL because `grapheme_x` does not exist.

- [ ] **Step 3: Implement grapheme x helpers**

Rules:

- Shaped path: convert target grapheme to target byte with `byte_index_for_grapheme()`, then sum clusters whose `byte_range.start < target_byte`.
- Fallback path: iterate `grapheme_slices(text)`, measure the first char of each grapheme with existing CJK/fullwidth heuristic; combining-only tail never becomes a separate position.
- `grapheme_at_x()` uses midpoint snapping per grapheme.

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p edit-plus-markdown grapheme_x_skips_combining_mark --lib
cargo test -p edit-plus-markdown grapheme_at_x --lib
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/layout/context.rs
git commit -m "feat: add grapheme x mapping for markdown preview"
```

## Task 4: 迁移 SelectionState 和 ViewPos 到 Grapheme 语义

**Files:**
- Modify: `crates/markdown/src/selection.rs`
- Modify: `crates/markdown/src/view.rs`
- Test: `crates/markdown/src/selection.rs`

**Interfaces:**
- Consumes: Task 1 grapheme slicing helpers
- Consumes: Task 3 `grapheme_at_x()` / `grapheme_x()`
- Produces: `ViewPos { flat_line_idx, grapheme_idx }`

- [ ] **Step 1: Write failing selected_text tests**

```rust
#[test]
fn selected_text_keeps_combining_grapheme_intact() {
    let line = test_flat_line("xe\u{0301}y");
    let mut selection = SelectionState::new();
    selection.anchor = Some(ViewPos { flat_line_idx: 0, grapheme_idx: 1 });
    selection.cursor = Some(ViewPos { flat_line_idx: 0, grapheme_idx: 2 });

    assert_eq!(selection.selected_text(&[line]), Some("e\u{0301}".to_string()));
}

#[test]
fn selected_text_keeps_zwj_emoji_intact() {
    let emoji = "👨\u{200D}👩\u{200D}👧";
    let line = test_flat_line(&format!("x{emoji}y"));
    let mut selection = SelectionState::new();
    selection.anchor = Some(ViewPos { flat_line_idx: 0, grapheme_idx: 1 });
    selection.cursor = Some(ViewPos { flat_line_idx: 0, grapheme_idx: 2 });

    assert_eq!(selection.selected_text(&[line]), Some(emoji.to_string()));
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test -p edit-plus-markdown selected_text_keeps --lib`

Expected: FAIL because `ViewPos` has `char_pos`, not `grapheme_idx`.

- [ ] **Step 3: Rename ViewPos field**

Change:

```rust
pub struct ViewPos {
    pub flat_line_idx: usize,
    pub char_pos: usize,
}
```

to:

```rust
pub struct ViewPos {
    pub flat_line_idx: usize,
    pub grapheme_idx: usize,
}
```

Update all comparisons to use `(flat_line_idx, grapheme_idx)`.

- [ ] **Step 4: Migrate functions**

Update:

- `hit_test()` uses `grapheme_at_x()`
- `line_range_at_pos()` returns `grapheme_count(text)`
- `select_all()` stores last line grapheme count
- `selected_text()` uses `byte_index_for_grapheme(text, grapheme_idx)`
- `highlights()` uses `grapheme_x()`

- [ ] **Step 5: Update `view.rs` typed protocol conversion**

All preview selection query/message conversions use `PreviewTextPosition` and `PreviewTextRange`:

```rust
PluginMessage::SetPreviewSelectionCursor(position) => {
    self.sel.cursor = position.map(|p| ViewPos {
        flat_line_idx: p.flat_line_idx,
        grapheme_idx: p.grapheme_idx,
    });
    Some(true)
}
```

and:

```rust
PluginResponse::PreviewTextPosition(
    self.sel.cursor.map(|p| PreviewTextPosition {
        flat_line_idx: p.flat_line_idx,
        grapheme_idx: p.grapheme_idx,
    })
)
```

Do not route Markdown preview selection through `PluginMessage::SetSelCursor`, `PluginMessage::SetSelAnchor`, `PluginQuery::SelCursor`, or `PluginResponse::Position`.

- [ ] **Step 6: Verify**

Run:

```bash
cargo test -p edit-plus-markdown selected_text_keeps --lib
cargo test -p edit-plus-markdown --lib -- selection
cargo check -p edit-plus-markdown
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/markdown/src/selection.rs crates/markdown/src/view.rs
git commit -m "fix: make markdown preview selection grapheme based"
```

## Task 5: 迁移 WordAtPos 到 Grapheme 边界

**Files:**
- Modify: `crates/markdown/src/selection.rs`
- Test: `crates/markdown/src/selection.rs`

**Interfaces:**
- Consumes: `grapheme_slices()`
- Produces: `word_at_pos(flat_lines: &[FlatLine], pos: ViewPos) -> (ViewPos, ViewPos)` where boundaries are grapheme indices.

- [ ] **Step 1: Write failing word boundary tests**

```rust
#[test]
fn word_at_pos_keeps_combining_letter_in_word() {
    let line = test_flat_line("xe\u{0301}y z");
    let (start, end) = word_at_pos(&[line], ViewPos { flat_line_idx: 0, grapheme_idx: 1 });

    assert_eq!(start, ViewPos { flat_line_idx: 0, grapheme_idx: 0 });
    assert_eq!(end, ViewPos { flat_line_idx: 0, grapheme_idx: 3 });
}

#[test]
fn word_at_pos_selects_single_emoji_grapheme() {
    let emoji = "👨\u{200D}👩\u{200D}👧";
    let line = test_flat_line(&format!("a {emoji} b"));
    let (start, end) = word_at_pos(&[line], ViewPos { flat_line_idx: 0, grapheme_idx: 2 });

    assert_eq!(end.grapheme_idx - start.grapheme_idx, 1);
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test -p edit-plus-markdown word_at_pos_keeps_combining --lib`

Expected: FAIL until `word_at_pos()` stops using `Vec<char>`.

- [ ] **Step 3: Classify grapheme by first non-mark codepoint**

Implement a helper:

```rust
fn grapheme_class(grapheme: &str) -> CharClass
```

Rules:

- If any char in grapheme is alphanumeric or `_`, classify as `Word`.
- Else if all chars are whitespace, classify as `Whitespace`.
- Else classify as `Punctuation`.

- [ ] **Step 4: Rewrite word expansion over grapheme slices**

Use `grapheme_slices(text)`; compare `grapheme_class(&text[range])`.

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p edit-plus-markdown word_at_pos_keeps_combining --lib
cargo test -p edit-plus-markdown word_at_pos_selects_single_emoji_grapheme --lib
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/markdown/src/selection.rs
git commit -m "fix: use grapheme word boundaries in markdown preview"
```

## Task 6: App Preview Keyboard Selection 使用 grapheme_count

**Files:**
- Modify: `crates/app/src/dispatch/editor.rs`
- Test: `crates/app/src/dispatch/editor.rs`

**Interfaces:**
- Consumes: `ui::plugin::FlatLine.grapheme_count`
- Consumes: `ui::plugin::PreviewTextPosition`
- Produces: Preview `ExtendLeft/Right/LineEnd/DocEnd` no longer calls `text.chars().count()` and no longer uses `PluginResponse::Position` for text selection.

- [ ] **Step 1: Write failing app-level test**

Use a stub preview plugin returning one `FlatLine { text: "xe\u{0301}y", grapheme_count: 3 }`, `PreviewSelectionCursor(Some(PreviewTextPosition { flat_line_idx: 0, grapheme_idx: 1 }))`, and record `SetPreviewSelectionCursor`.

```rust
#[test]
fn preview_extend_to_line_end_uses_grapheme_count() {
    use ui::plugin::PreviewTextPosition;

    let mut app = App::new(None);
    app.push_preview_selection_stub_for_test(
        "xe\u{0301}y",
        3,
        Some(PreviewTextPosition { flat_line_idx: 0, grapheme_idx: 1 }),
    );

    app.dispatch_edit_command(EditCommand::ExtendToLineEnd, test_event_loop());

    assert_eq!(
        app.preview_stub_last_cursor_for_test(),
        Some(PreviewTextPosition { flat_line_idx: 0, grapheme_idx: 3 })
    );
}
```

- [ ] **Step 2: Run test to verify failure**

Run: `cargo test -p edit-plus-app preview_extend_to_line_end_uses_grapheme_count --lib`

Expected: FAIL because current code uses `fl.text.chars().count()` and would produce 4.

- [ ] **Step 3: Replace app-side char counts**

In preview-only branch of `dispatch_edit_command()`:

- `ExtendLeft`: previous line length uses `fl.grapheme_count`
- `ExtendRight`: current line length uses `fl.grapheme_count`
- `ExtendToLineEnd`: target uses `fl.grapheme_count`
- `ExtendToDocEnd`: last target uses `fl.grapheme_count`
- `PreviewSelectionCursor` query replaces `SelCursor`
- `SetPreviewSelectionCursor` message replaces `SetSelCursor`

- [ ] **Step 4: Rename local variables**

Rename local variables:

- `cursor_position` -> `PreviewTextPosition`
- `new_cp` -> `new_grapheme_idx`
- `line_len` -> `line_grapheme_count`
- `last_char` -> `last_grapheme_idx`

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p edit-plus-app preview_extend_to_line_end_uses_grapheme_count --lib
cargo test -p edit-plus-app --lib -- preview
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/dispatch/editor.rs crates/app/src/app_tests.rs
git commit -m "fix: use grapheme counts for preview keyboard selection"
```

## Task 7: App Preview Mouse Selection 命名与 Grapheme 协议对齐

**Files:**
- Modify: `crates/app/src/dispatch/mouse.rs`
- Test: `crates/app/src/dispatch/mouse.rs`

**Interfaces:**
- Consumes: `PluginQuery::PreviewHitTest` returns `PluginResponse::PreviewTextPosition`.
- Consumes: `PluginQuery::PreviewWordAt` and `PluginQuery::PreviewLineRangeAt` return `PluginResponse::PreviewTextRange`.
- Produces: Mouse click/drag/double-click/triple-click pass `PreviewTextPosition` through without tuple conversion.

- [ ] **Step 1: Write failing drag test with combining text**

Use a stub preview plugin whose `PreviewHitTest` returns `PreviewTextPosition { flat_line_idx: 0, grapheme_idx: 2 }` when dragging after `e\u{0301}`, and record `SetPreviewSelectionCursor`.

```rust
#[test]
fn preview_drag_passes_grapheme_position_to_plugin() {
    use ui::plugin::PreviewTextPosition;

    let mut app = App::new(None);
    app.push_preview_hit_test_stub_for_test(vec![
        PreviewTextPosition { flat_line_idx: 0, grapheme_idx: 1 },
        PreviewTextPosition { flat_line_idx: 0, grapheme_idx: 2 },
    ]);

    app.dispatch_preview_mouse_press_for_test(10.0, 10.0);
    app.dispatch_preview_mouse_drag_for_test(30.0, 10.0);

    assert_eq!(
        app.preview_stub_last_cursor_for_test(),
        Some(PreviewTextPosition { flat_line_idx: 0, grapheme_idx: 2 })
    );
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p edit-plus-app preview_drag_passes_grapheme_position_to_plugin --lib`

Expected: PASS may already happen behaviorally; if so, keep the test as a guard and complete the naming cleanup.

- [ ] **Step 3: Replace preview mouse queries/messages**

In preview mouse path:

- `PluginQuery::HitTest` -> `PluginQuery::PreviewHitTest`
- `PluginQuery::WordAtPos` -> `PluginQuery::PreviewWordAt`
- `PluginQuery::LineRangeAtPos` -> `PluginQuery::PreviewLineRangeAt`
- `PluginMessage::SetSelAnchor` -> `PluginMessage::SetPreviewSelectionAnchor`
- `PluginMessage::SetSelCursor` -> `PluginMessage::SetPreviewSelectionCursor`
- `PluginResponse::Position` -> `PluginResponse::PreviewTextPosition`
- `PluginResponse::PositionPair` -> `PluginResponse::PreviewTextRange`

- [ ] **Step 4: Rename locals in preview mouse path**

Replace local names:

- `preview_pos` -> `preview_text_position`
- `hit_pos` -> `hit_grapheme_pos`
- `line_start` / `line_end` remain acceptable for range endpoints.

- [ ] **Step 5: Verify double/triple click still compiles**

Run:

```bash
cargo test -p edit-plus-app --lib -- mouse
cargo check -p edit-plus-app
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/dispatch/mouse.rs
git commit -m "refactor: align preview mouse selection with grapheme protocol"
```

## Task 8: Markdown Preview Integration Tests

**Files:**
- Modify: `crates/markdown/src/view.rs`
- Test: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: Tasks 1-7.
- Produces: End-to-end plugin query/message coverage for preview-only selection.

- [ ] **Step 1: Add copy integration test**

```rust
#[test]
fn markdown_preview_copy_selection_keeps_combining_grapheme() {
    use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, PreviewTextPosition, ViewPlugin};

    let doc = StubDoc::new("xe\u{0301}y");
    let mut view = MarkdownView::new(doc.text.clone(), 1);
    render_preview_once(&mut view, &doc);

    view.handle_message(
        PluginMessage::SetPreviewSelectionAnchor(Some(PreviewTextPosition {
            flat_line_idx: 0,
            grapheme_idx: 1,
        })),
        &mut doc.clone_for_mut(),
    );
    view.handle_message(
        PluginMessage::SetPreviewSelectionCursor(Some(PreviewTextPosition {
            flat_line_idx: 0,
            grapheme_idx: 2,
        })),
        &mut doc.clone_for_mut(),
    );

    let selected = match view.query(PluginQuery::SelectedText, &doc) {
        PluginResponse::String(text) => text,
        other => panic!("expected selected string, got {other:?}"),
    };

    assert_eq!(selected, "e\u{0301}");
}
```

- [ ] **Step 2: Add highlight integration test**

```rust
#[test]
fn markdown_preview_selection_highlight_width_covers_whole_zwj_emoji() {
    use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, PreviewTextPosition, ViewPlugin};

    let emoji = "👨\u{200D}👩\u{200D}👧";
    let doc = StubDoc::new(&format!("x{emoji}y"));
    let mut view = MarkdownView::new(doc.text.clone(), 1);
    render_preview_once(&mut view, &doc);

    view.handle_message(
        PluginMessage::SetPreviewSelectionAnchor(Some(PreviewTextPosition {
            flat_line_idx: 0,
            grapheme_idx: 1,
        })),
        &mut doc.clone_for_mut(),
    );
    view.handle_message(
        PluginMessage::SetPreviewSelectionCursor(Some(PreviewTextPosition {
            flat_line_idx: 0,
            grapheme_idx: 2,
        })),
        &mut doc.clone_for_mut(),
    );

    let dl = match view.query(PluginQuery::SelectionHighlights([1.0, 0.0, 0.0, 1.0]), &doc) {
        PluginResponse::DrawList(dl) => dl,
        other => panic!("expected draw list, got {other:?}"),
    };

    assert!(!dl.cmds.is_empty());
}
```

- [ ] **Step 3: Run tests**

Run:

```bash
cargo test -p edit-plus-markdown markdown_preview_copy_selection_keeps_combining_grapheme --lib
cargo test -p edit-plus-markdown markdown_preview_selection_highlight_width_covers_whole_zwj_emoji --lib
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/markdown/src/view.rs
git commit -m "test: cover grapheme preview selection integration"
```

## Task 9: 清理协议注释与旧 char 命名

**Files:**
- Modify: `crates/ui/src/plugin.rs`
- Modify: `crates/markdown/src/selection.rs`
- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/app/src/dispatch/editor.rs`
- Modify: `crates/app/src/dispatch/mouse.rs`
- Modify: `docs/plans/2026-06-27-preview-selection-grapheme-plan.md`

**Interfaces:**
- Produces: preview selection 代码中无 editor-facing `char_pos` 命名，且 Markdown preview selection 不再调用旧 tuple 协议。

- [ ] **Step 1: Search stale names**

Run:

```bash
rg -n "char_pos|char_count|last_char|new_cp|\\bcp\\b|source_bytes_by_visual_char" crates/markdown/src/selection.rs crates/markdown/src/view.rs crates/app/src/dispatch/editor.rs crates/app/src/dispatch/mouse.rs crates/ui/src/plugin.rs
```

Expected: No preview selection-facing stale names remain. `source_bytes_by_visual_char` may still appear in WYSIWYG layout files covered by the separate WYSIWYG plan, but not in `selection.rs`.

- [ ] **Step 2: Search old tuple protocol usage in preview paths**

Run:

```bash
rg -n "SetSelCursor|SetSelAnchor|PluginQuery::SelCursor|PluginQuery::HitTest|WordAtPos|LineRangeAtPos|PluginResponse::Position\\(" crates/markdown/src/view.rs crates/app/src/dispatch/editor.rs crates/app/src/dispatch/mouse.rs
```

Expected: Markdown preview selection paths do not use these variants. Remaining uses must be for non-preview-text plugin paths and documented inline.

- [ ] **Step 3: Update comments**

In `crates/ui/src/plugin.rs`, document old tuple variants as legacy/non-preview-text:

- `SetSelCursor`: legacy non-preview-text selection position.
- `SetSelAnchor`: legacy non-preview-text selection anchor.
- `SelCursor`: legacy position query, not for Markdown preview text selection.
- `HitTest`: legacy position hit-test, not for Markdown preview text selection.
- `WordAtPos`: legacy tuple word query.
- `LineRangeAtPos`: legacy tuple line query.

Document new preview variants as the required path:

- `SetPreviewSelectionCursor`: `PreviewTextPosition`.
- `SetPreviewSelectionAnchor`: `PreviewTextPosition`.
- `PreviewSelectionCursor`: returns `PreviewTextPosition`.
- `PreviewHitTest`: returns `PreviewTextPosition`.
- `PreviewWordAt`: returns `PreviewTextRange`.
- `PreviewLineRangeAt`: returns `PreviewTextRange`.

- [ ] **Step 4: Add implementation note to this document**

Append a short "Implemented Notes" section listing:

- Actual helper names.
- Any retained char-based search path.
- Any plugin that intentionally remains on legacy tuple variants.

- [ ] **Step 5: Final targeted verification**

Run:

```bash
cargo fmt --all -- --check
cargo test -p edit-plus-markdown --lib -- selection
cargo test -p edit-plus-markdown --lib -- markdown_preview
cargo test -p edit-plus-app --lib -- preview
cargo check -p edit-plus-app
```

Expected: PASS.

- [ ] **Step 6: Major verification**

Run: `./scripts/verify.sh`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/ui/src/plugin.rs crates/markdown/src crates/app/src/dispatch docs/plans/2026-06-27-preview-selection-grapheme-plan.md
git commit -m "docs: record preview selection grapheme plan"
```

---

## Verification Matrix

| Area | Command | Expected |
|------|---------|----------|
| Grapheme helper | `cargo test -p edit-plus-markdown grapheme_count_handles --lib` | PASS |
| Byte lookup | `cargo test -p edit-plus-markdown byte_index_for_grapheme --lib` | PASS |
| Preview protocol | `cargo test -p edit-plus-markdown preview_protocol_reports_grapheme_count_and_typed_position --lib` | PASS |
| X mapping | `cargo test -p edit-plus-markdown grapheme_x_skips_combining_mark --lib` | PASS |
| Selection slicing | `cargo test -p edit-plus-markdown selected_text_keeps --lib` | PASS |
| Word boundary | `cargo test -p edit-plus-markdown word_at_pos_keeps_combining --lib` | PASS |
| App keyboard preview | `cargo test -p edit-plus-app preview_extend_to_line_end_uses_grapheme_count --lib` | PASS |
| App mouse preview | `cargo test -p edit-plus-app preview_drag_passes_grapheme_position_to_plugin --lib` | PASS |
| Integration | `cargo test -p edit-plus-markdown --lib -- markdown_preview` | PASS |
| Formatting | `cargo fmt --all -- --check` | PASS |
| Compile | `cargo check -p edit-plus-app` | PASS |
| Full verification | `./scripts/verify.sh` | PASS |

## Manual Acceptance

- [ ] Markdown preview 中选择 `e\u{0301}` 时，高亮覆盖整个用户感知字符，复制结果也是完整 `e\u{0301}`。
- [ ] Markdown preview 中拖拽跨过 `👨‍👩‍👧`，高亮不能停在 emoji 内部。
- [ ] Shift+Right 在 `e\u{0301}` 前按一次，选择整个 grapheme。
- [ ] Shift+Left 从 `e\u{0301}` 后按一次，选择整个 grapheme。
- [ ] 双击 `xéy` 中的组合字符单词，选中整词 `xéy`。
- [ ] 三击任意含 emoji 的行，选中整行，不截断 emoji。
- [ ] Copy/Cut 在 preview-only 模式只复制完整 grapheme，不产生非法 UTF-8 或孤立 combining mark。

## Known Follow-Up After This Plan

- `crates/markdown/src/search.rs` 的 search highlight 仍按 char index 计算匹配矩形；若搜索高亮也要求严格 grapheme-safe，应另开 search-specific 计划。
- WYSIWYG editor source-byte 光标、marker 展开、selection byte 映射由 `2026-06-27-mdeditor-grapheme-cursor-state-plan.md` 覆盖。
- Mindmap plugin 的 `SetSelCursor(line, col)` 实际转换到 byte focus，不使用 Markdown preview text selection。该 legacy tuple 可保留给 mindmap，但 Markdown preview 不能继续调用它。
