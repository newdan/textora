# Mdeditor Grapheme Cursor State Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 统一 Markdown WYSIWYG 编辑态的光标、选择、命中测试、编辑后同步和 grapheme 边界语义，消除 byte / char / grapheme / glyph cluster 混用导致的光标漂移。

**Architecture:** `core` 继续作为文本内容与 grapheme 光标边界的事实来源；`app` 负责输入调度、`DocumentView` 光标回读和 plugin 状态同步；`markdown` plugin 负责 WYSIWYG 视觉布局、source-to-visual 映射、光标绘制和选择高亮。修复方向不是补单点边界判断，而是把 Markdown 编辑态的内部坐标从 `char_pos` 迁移到 `grapheme_pos`，并保证每次 byte 写入 `DocumentView` 后都回读 snapped cursor byte 再同步给 plugin。

**Tech Stack:** Rust workspace；`edit-plus-core` 提供 `ByteIndex` / `UniCharOffset` / `CursorMovement::Grapheme`；`edit-plus-app` 提供 `DocumentView`、dispatch、mouse、WYSIWYG host sync；`edit-plus-markdown` 提供 parser/builder/layout/view/edit/selection；验证使用 `cargo test`、`cargo check`、`cargo fmt`，重大修改后执行 `./scripts/verify.sh`。

## Global Constraints

- 全程遵守 `AGENTS.md`：中文沟通，需求不明先问清楚，遇 bug 先写复现测试再修。
- 单任务修改超过 3 个文件必须拆分为子任务；每次提交前必须确保编译通过。
- `crates/ui` 只能定义纯数据协议，绝对禁止依赖或访问 `crates/app` 状态结构体。
- 新增跨层接口只能放在 `crates/ui/src/plugin.rs` 的纯数据协议，或 `crates/markdown` 内部纯数据结构。
- WYSIWYG 光标移动不得触发整篇 Markdown 全量 parse/build；源码变化可以继续走现有 source dirty 路径。
- 任何用户可见光标位置必须落在 UTF-8 char boundary 且落在 UAX #29 grapheme boundary。
- 选择范围的事实来源必须能回到 source byte range；视觉高亮只能是这个 range 的投影。
- 命名必须精准自解释，禁止新增 `data` / `info` / `temp` / `flag` 等宽泛名称。

---

## 当前状态全景

### 1. App 级 WYSIWYG 状态

| 状态 | 位置 | 当前职责 | 风险 |
|------|------|----------|------|
| `App::wysiwyg_preferred_x` | `crates/app/src/app.rs` / `dispatch/wysiwyg.rs` | 上下移动时保存 sticky x | 依赖 plugin 返回的视觉 x；如果 plugin 的 byte->visual 映射错误，上下移动继续放大错误 |
| `App::wysiwyg_recursing` | `crates/app/src/app.rs` / `dispatch/wysiwyg.rs` | 防止 augmented enter/backspace 递归拦截 | 状态合理，但执行后必须调用统一 sync |
| `MouseState::down_offset` | `crates/app/src/mouse.rs` | 普通编辑器 drag selection 的 unichar anchor | WYSIWYG drag 走另一套 byte -> `SetSelCursorByte`，两套 selection 语义不一致 |
| `sync_wysiwyg_plugin_state()` | `crates/app/src/app_renderer.rs` | 推送 source text、generation、cursor byte 给 plugin | 只推送 `DocumentView` 当前 byte；如果之前 plugin 给了 mid-grapheme byte，必须先经 `DocumentView` snap 后回读 |

### 2. DocumentView 状态

| 状态 | 位置 | 当前职责 | 风险 |
|------|------|----------|------|
| `TextBuffer` cursor | `crates/core/src/buffer/*` | 内容、undo/redo、grapheme cursor 的事实来源 | 核心层已有 grapheme 能力，应继续复用 |
| `CursorState::offset` | `crates/app/src/document_view/cursor.rs` | `DocumentView` 缓存的 byte cursor | 必须始终等于 `tb.cursor_offset()` |
| `CursorState::selection_anchor` | `crates/app/src/document_view/cursor.rs` | 普通编辑器 selection anchor，单位 byte | WYSIWYG selection 不直接使用它，导致选择事实来源分裂 |
| `LineIndex::unichar_offsets` | `crates/app/src/line_index.rs` | document-level grapheme offset 与 line 映射 | 可作为 WYSIWYG source byte <-> grapheme 边界工具的参考 |
| `cursor_column()` | `crates/app/src/document_view/mod.rs` | 返回行内 byte offset | 函数名像 column，实际是 byte；无 advance_cache 时 Home 仍可能把 byte 当 grapheme 步数 |

### 3. Plugin 协议状态

| 消息 / 查询 | 位置 | 当前职责 | 风险 |
|-------------|------|----------|------|
| `SetCursorByte(usize)` | `crates/ui/src/plugin.rs` | host 通知 plugin 当前源码 byte cursor | 只传 byte，不表达 grapheme 边界或 affinity |
| `CursorScreenPos(usize)` | `crates/ui/src/plugin.rs` | source byte -> cursor rect | plugin 内部用 char map 找 visual char |
| `HitTestByte` | `crates/ui/src/plugin.rs` | screen point -> source byte | 当前中间坐标是 visual char |
| `VisualMoveWysiwyg` | `crates/ui/src/plugin.rs` | WYSIWYG 键盘视觉移动 | plugin 返回 byte，host 再移动 `DocumentView` |
| `SetSelCursorByte` / `SetSelAnchorByte` | `crates/ui/src/plugin.rs` | WYSIWYG drag selection | byte 进入 plugin 后变成 `ViewPos { char_pos }` |

### 4. MarkdownEditorView 状态

| 状态 | 位置 | 当前职责 | 风险 |
|------|------|----------|------|
| `MarkdownEditorView::source` | `crates/markdown/src/view.rs` | plugin 缓存的完整 Markdown source | 与 `DocumentView` generation 同步；source dirty 可全量 rebuild |
| `MarkdownEditorView::generation` | `crates/markdown/src/view.rs` | 判断 source 是否 stale | 光标移动不改 generation，因此 cursor-only path 必须可靠 |
| `PreviewEngine::dirty` | `crates/markdown/src/view.rs` | `Clean` / `SourceChanged` / `CursorMoved` 等布局脏状态 | `CursorMoved` 局部重排必须覆盖旧 cursor block 和新 cursor block |
| `PreviewEngine::edit_ctx` | `crates/markdown/src/view.rs` | 当前 WYSIWYG cursor byte | 只有 byte，没有 grapheme position、视觉 affinity、selection 状态 |
| `PreviewEngine::edit_source` | `crates/markdown/src/view.rs` | materialize span 时读取原始 marker | 正确，但 materialize map 当前按 char 建表 |
| `PreviewEngine::cursor_visible` | `crates/markdown/src/view.rs` | blink phase | 只影响绘制，不应影响布局 |
| `PreviewEngine::sel` | `crates/markdown/src/view.rs` | plugin 自己的 selection | 单位是 `ViewPos { char_pos }`，不是 grapheme |
| `PreviewEngine::cached_dl` / `cached_vertices` | `crates/markdown/src/view.rs` | render cache | cursor/selection/source map 改变时必须失效 |
| `PreviewEngine::scroll_y` | `crates/markdown/src/view.rs` | plugin 自渲染滚动位置 | 已是 WYSIWYG 的真实滚动状态；host 不应同时改普通 viewport |

### 5. LazyLayout / FlatLine 状态

| 状态 | 位置 | 当前职责 | 风险 |
|------|------|----------|------|
| `LazyLayout::flat_lines` | `crates/markdown/src/layout/types.rs` | 可视阅读顺序行 | selection 和 hit-test 都依赖它 |
| `LazyLayout::block_line_map` | `crates/markdown/src/layout/types.rs` | flat line -> source block line | 折行后 disambiguation 依赖它 |
| `LazyLayout::line_byte_offsets` | `crates/markdown/src/layout/types.rs` | source line 起始 byte | source map fallback 依赖它 |
| `FlatLineSourceMap::source_bytes_by_visual_char` | `crates/markdown/src/layout/types.rs` | visual char index -> source byte | 核心问题：单位是 char，不是 grapheme |
| `LaidOutLine::source_bytes_by_visual_char` | `crates/markdown/src/layout/types.rs` | materialized line map | 同上 |
| `FlatLine::shaped` | `crates/markdown/src/layout/types.rs` | HarfBuzz glyph cluster | 渲染单位是 cluster，但交互单位是 char |
| `y_delta` / `estimated_positions` | `crates/markdown/src/layout/types.rs` | lazy layout 高度修正 | cursor-only 展开 marker 不应意外移动后续 block |

### 6. Markdown edit / selection 状态

| 状态 | 位置 | 当前职责 | 风险 |
|------|------|----------|------|
| `EditContext { cursor_byte }` | `crates/markdown/src/edit.rs` | 决定 active inline span / block marker | byte-only，不表达 grapheme 边界 |
| `MaterializedLine::visual_char_to_source_byte` | `crates/markdown/src/edit.rs` | materialized text 到 source byte 的映射 | 字段名和契约明确是 char；ZWJ/combining 会错 |
| `ViewPos { flat_line_idx, char_pos }` | `crates/markdown/src/selection.rs` | preview/editor selection 坐标 | selection 也不是 grapheme-safe |
| `SelectionState::selected_text()` | `crates/markdown/src/selection.rs` | 从 flat lines 提取选中文本 | 用 `char_indices().nth(char_pos)`，会切错 grapheme |
| `SelectionState::highlights()` | `crates/markdown/src/selection.rs` | 根据 `char_x` 画高亮 | 若 char_pos 落在 grapheme 内，高亮和光标会分离 |

## 根因结论

Markdown WYSIWYG 编辑态尚未彻底落地 grapheme。当前实际链路是：

```text
screen x
  -> shaped glyph cluster midpoint
  -> visual char index
  -> source byte
  -> DocumentView cursor_move_to_offset()
  -> TextBuffer snap
  -> plugin SetCursorByte(original or snapped byte)
  -> visual char index
  -> cursor rect / selection highlight
```

这个链路里至少有三种坐标空间：glyph cluster、Rust char index、source byte。`core` 中已经存在 `CursorMovement::Grapheme` 和 `UniCharOffset`，但 WYSIWYG plugin 没有使用同一套边界模型。因此在以下输入中会出现光标、编辑后文字、选择关系不稳定：

- NFD combining mark：`e\u{0301}`
- ZWJ emoji：`👨‍👩‍👧`
- emoji + variation selector：`✈️`
- inline marker 展开/折叠：`**text**`、`` `code` ``
- active block marker：`# `、`- `、`> `、`- [ ] `
- 软折行边界与上下移动 sticky x
- drag selection 从 folded layout 进入 expanded layout

---

## Target Model

最终统一为四类强约束状态：

```text
SourceByte        = absolute UTF-8 byte offset in source
SourceGrapheme    = document-level UAX #29 grapheme offset
VisualGraphemePos = flat_line_idx + line-local grapheme index
PixelRect         = x/y/w/h in plugin document coordinates
```

所有用户交互只能走下面的可逆投影：

```text
SourceByte <-> SourceGrapheme <-> VisualGraphemePos <-> PixelRect
```

其中 `SourceByte` 仍是跨层协议和 `DocumentView` 兼容的外部单位，但 plugin 内部不得再用 `char_pos` 作为编辑态事实坐标。

---

## File Structure

- Modify: `crates/markdown/src/grapheme_map.rs`
  - 新增文件。负责 text/source map 的 grapheme 边界建表、byte snapping、x hit-test 辅助。
- Modify: `crates/markdown/src/edit.rs`
  - 把 `MaterializedLine` 的 visual map 从 char index 迁移为 grapheme index。
- Modify: `crates/markdown/src/layout/context.rs`
  - 把 `char_at_x` / `char_x` 替换或包裹为 grapheme 版本。
- Modify: `crates/markdown/src/layout/types.rs`
  - `FlatLineSourceMap`、`LaidOutLine`、`FlatLine` 的映射字段改为 grapheme 语义。
- Modify: `crates/markdown/src/layout/block.rs`
  - materialized map 切片、active marker prepend 改为 grapheme map。
- Modify: `crates/markdown/src/selection.rs`
  - `ViewPos.char_pos` 迁移为 `grapheme_pos`，选择文本/高亮基于 grapheme。
- Modify: `crates/markdown/src/view.rs`
  - `HitTestByte`、`CursorScreenPos`、`VisualMoveWysiwyg`、selection byte sync 全部使用 grapheme map。
- Modify: `crates/app/src/dispatch/wysiwyg.rs`
  - host 写入 `DocumentView` 后回读 snapped cursor byte，再 `SetCursorByte(snapped_byte)`。
- Modify: `crates/app/src/dispatch/mouse.rs`
  - WYSIWYG 点击/drag 的二阶段 hit-test 使用 snapped byte，同步 selection cursor。
- Optional Modify: `crates/ui/src/plugin.rs`
  - 仅在需要兼容期协议命名时新增纯数据别名；不要引入 app 类型。

---

## Task 1: 建立 WYSIWYG Grapheme Map 基础设施

**Files:**
- Create: `crates/markdown/src/grapheme_map.rs`
- Modify: `crates/markdown/src/lib.rs`
- Test: `crates/markdown/src/grapheme_map.rs`

**Interfaces:**
- Produces: `VisualGraphemeMap`, `VisualGraphemeIndex`, `build_visual_grapheme_map(text: &str, source_bytes_by_char: &[usize]) -> VisualGraphemeMap`
- Produces: `VisualGraphemeMap::source_byte_at(grapheme_index: usize) -> Option<usize>`
- Produces: `VisualGraphemeMap::byte_to_grapheme(source_byte: usize) -> Option<usize>`
- Consumes: source byte sentinel maps currently produced by `materialize_line()`

- [ ] **Step 1: Write failing grapheme map tests**

Add tests covering ASCII, CJK, NFD combining, ZWJ emoji, variation selector:

```rust
#[test]
fn grapheme_map_treats_nfd_combining_as_one_position() {
    let text = "xe\u{0301}y";
    let source_by_char = vec![0, 1, 2, 4, 5];
    let map = build_visual_grapheme_map(text, &source_by_char);

    assert_eq!(map.len(), 4);
    assert_eq!(map.source_byte_at(1), Some(1));
    assert_eq!(map.source_byte_at(2), Some(4));
    assert_eq!(map.byte_to_grapheme(2), Some(1));
}

#[test]
fn grapheme_map_treats_zwj_emoji_as_one_position() {
    let emoji = "👨\u{200D}👩\u{200D}👧";
    let text = format!("x{emoji}y");
    let mut source_by_char = Vec::new();
    for (byte, _) in text.char_indices() {
        source_by_char.push(byte);
    }
    source_by_char.push(text.len());

    let map = build_visual_grapheme_map(&text, &source_by_char);

    assert_eq!(map.len(), 4);
    assert_eq!(map.source_byte_at(1), Some(1));
    assert_eq!(map.source_byte_at(2), Some(1 + emoji.len()));
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test -p edit-plus-markdown grapheme_map_ --lib`

Expected: FAIL because `grapheme_map` module/functions do not exist.

- [ ] **Step 3: Implement grapheme map with existing UCD helpers**

Use `core::unicode::{ucd_grapheme_cluster_lookup, ucd_grapheme_cluster_joins, ucd_grapheme_cluster_joins_done}` instead of adding a dependency.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VisualGraphemeMap {
    source_bytes_by_grapheme: Vec<usize>,
}

impl VisualGraphemeMap {
    pub(crate) fn len(&self) -> usize {
        self.source_bytes_by_grapheme.len()
    }

    pub(crate) fn source_byte_at(&self, grapheme_index: usize) -> Option<usize> {
        self.source_bytes_by_grapheme.get(grapheme_index).copied()
    }

    pub(crate) fn byte_to_grapheme(&self, source_byte: usize) -> Option<usize> {
        self.source_bytes_by_grapheme
            .iter()
            .enumerate()
            .min_by_key(|(_, mapped_byte)| mapped_byte.abs_diff(source_byte))
            .map(|(idx, _)| idx)
    }

    pub(crate) fn as_slice(&self) -> &[usize] {
        &self.source_bytes_by_grapheme
    }
}
```

- [ ] **Step 4: Verify**

Run: `cargo test -p edit-plus-markdown grapheme_map_ --lib`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/grapheme_map.rs crates/markdown/src/lib.rs
git commit -m "test: add markdown visual grapheme map"
```

## Task 2: 迁移 MaterializedLine 和 FlatLineSourceMap 到 Grapheme 语义

**Files:**
- Modify: `crates/markdown/src/edit.rs`
- Modify: `crates/markdown/src/layout/types.rs`
- Modify: `crates/markdown/src/layout/block.rs`
- Test: `crates/markdown/src/edit.rs`
- Test: `crates/markdown/src/layout/types.rs`

**Interfaces:**
- Consumes: `VisualGraphemeMap` from Task 1
- Produces: `MaterializedLine::visual_grapheme_to_source_byte(&self, visual_grapheme: usize) -> Option<usize>`
- Produces: `FlatLineSourceMap::source_bytes_by_visual_grapheme: Vec<usize>`

- [ ] **Step 1: Write failing materialization tests**

Add a test where expanded bold contains NFD combining:

```rust
#[test]
fn materialized_line_maps_combining_grapheme_to_single_source_position() {
    let source = "**e\u{0301}**";
    let line_text = "e\u{0301}";
    let spans = vec![make_span(0, line_text.len(), 0, source.len(), InlineStyle::Bold)];
    let ctx = EditContext { cursor_byte: 3 };

    let line = materialize_line(line_text, &spans, source, Some(&ctx));

    assert_eq!(line.text, source);
    assert_eq!(line.visual_grapheme_to_source_byte(0), Some(0));
    assert_eq!(line.visual_grapheme_to_source_byte(2), Some(2));
    assert_eq!(line.visual_grapheme_to_source_byte(3), Some(2 + "e\u{0301}".len()));
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test -p edit-plus-markdown materialized_line_maps_combining --lib`

Expected: FAIL because `visual_grapheme_to_source_byte` does not exist.

- [ ] **Step 3: Rename and migrate source map fields**

Rename fields and comments:

```rust
pub struct MaterializedLine {
    pub text: String,
    pub spans: Vec<MaterializedSpan>,
    visual_grapheme_to_source_byte: Vec<usize>,
}

pub struct FlatLineSourceMap {
    pub flat_idx: usize,
    pub source_bytes_by_visual_grapheme: Vec<usize>,
}
```

Do not keep ambiguous aliases named `visual_char`.

- [ ] **Step 4: Build grapheme maps when materializing**

Keep the existing char-level temporary map local to `materialize_line()`, then immediately convert it through `build_visual_grapheme_map()`. The public stored map must be grapheme-based.

- [ ] **Step 5: Update wrapped-line slicing**

In `layout/block.rs`, replace char-count slicing:

```rust
let char_start = text[..seg_start].chars().count();
let seg_chars = w.text.chars().count();
map[char_start..=char_start + seg_chars].to_vec()
```

with a helper that slices by byte segment boundaries through grapheme source byte positions. The helper must keep the one-past-end sentinel.

- [ ] **Step 6: Verify**

Run:

```bash
cargo test -p edit-plus-markdown materialized_line_maps_combining --lib
cargo test -p edit-plus-markdown heading_active_marker_source_map_correct --lib
cargo test -p edit-plus-markdown list_item_active_marker_source_map_correct --lib
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/markdown/src/edit.rs crates/markdown/src/layout/types.rs crates/markdown/src/layout/block.rs
git commit -m "refactor: use grapheme source maps in markdown layout"
```

## Task 3: 迁移 Hit-Test 和 Cursor Rect 到 Grapheme 坐标

**Files:**
- Modify: `crates/markdown/src/layout/context.rs`
- Modify: `crates/markdown/src/view.rs`
- Test: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: `FlatLineSourceMap::source_bytes_by_visual_grapheme`
- Produces: `grapheme_at_x(flat_line: &FlatLine, rel_x: f32) -> usize`
- Produces: `grapheme_x(flat_line: &FlatLine, visual_grapheme: usize) -> f32`

- [ ] **Step 1: Write failing hit-test roundtrip tests**

Add WYSIWYG tests:

```rust
#[test]
fn wysiwyg_hit_test_roundtrips_combining_grapheme() {
    let mut view = make_view("**e\u{0301}**");
    view.engine_mut().handle_set_cursor_byte(3);

    let (x, y, _w, h) = view.engine().cursor_screen_pos().expect("cursor should resolve");
    let hit = view.engine().hit_test_byte(x, y + h * 0.5, 0.0, 0.0);

    assert_eq!(hit, Some(3));
}

#[test]
fn wysiwyg_hit_test_roundtrips_zwj_emoji() {
    let emoji = "👨\u{200D}👩\u{200D}👧";
    let source = format!("**{emoji}**");
    let mut view = make_view(&source);
    view.engine_mut().handle_set_cursor_byte(2 + emoji.len());

    let (x, y, _w, h) = view.engine().cursor_screen_pos().expect("cursor should resolve");
    let hit = view.engine().hit_test_byte(x, y + h * 0.5, 0.0, 0.0);

    assert_eq!(hit, Some(2 + emoji.len()));
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test -p edit-plus-markdown wysiwyg_hit_test_roundtrips --lib`

Expected: at least one FAIL under current char-based mapping.

- [ ] **Step 3: Replace char helpers**

In `layout/context.rs`, keep compatibility wrappers only if needed, but WYSIWYG paths must call grapheme helpers:

```rust
pub(crate) fn grapheme_at_x(flat_line: &FlatLine, rel_x: f32) -> usize
pub(crate) fn grapheme_x(flat_line: &FlatLine, visual_grapheme: usize) -> f32
```

For shaped lines, iterate `shaped.clusters` and count grapheme starts before `cluster.byte_range.start`. For fallback, iterate grapheme clusters, not `text.chars()`.

- [ ] **Step 4: Update view queries**

Update these methods to use grapheme names and maps:

- `cursor_screen_pos()`
- `hit_test_byte()`
- `byte_from_flat_line_and_visual_char()` -> `byte_from_flat_line_and_visual_grapheme()`
- `flat_line_and_x_for_byte()`
- `find_flat_and_char_for_byte()` -> `find_flat_and_grapheme_for_byte()`
- `char_offset_from_byte()` -> `grapheme_offset_from_byte()`

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p edit-plus-markdown wysiwyg_hit_test_roundtrips --lib
cargo test -p edit-plus-markdown hit_test_byte_roundtrip_inside_cjk_bold_span --lib
cargo test -p edit-plus-markdown two_phase_hit_test_from_folded_to_expanded_span --lib
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/markdown/src/layout/context.rs crates/markdown/src/view.rs
git commit -m "fix: use grapheme hit testing in markdown editor"
```

## Task 4: 迁移 WYSIWYG 视觉导航到 Grapheme 步进

**Files:**
- Modify: `crates/markdown/src/view.rs`
- Test: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: `find_flat_and_grapheme_for_byte()`
- Produces: `visual_move()` semantics: Left/Right move by one visual grapheme, not one Rust char or byte.

- [ ] **Step 1: Replace current old-contract test**

Current test `visual_move_within_single_line_expanded_span` asserts "advance one byte". Replace it with ASCII-equivalent grapheme wording:

```rust
#[test]
fn visual_move_right_advances_one_grapheme_in_expanded_span() {
    let mut v = make_view("**bold**");
    v.engine_mut().handle_set_cursor_byte(3);

    let result = v.engine().visual_move(3, MoveDirection::Right, None);

    assert_eq!(result, Some(4));
}
```

- [ ] **Step 2: Add failing combining and ZWJ navigation tests**

```rust
#[test]
fn visual_move_right_skips_combining_mark() {
    let source = "**e\u{0301}x**";
    let mut v = make_view(source);
    v.engine_mut().handle_set_cursor_byte(2);

    let result = v.engine().visual_move(2, MoveDirection::Right, None);

    assert_eq!(result, Some(2 + "e\u{0301}".len()));
}

#[test]
fn visual_move_right_skips_zwj_emoji_cluster() {
    let emoji = "👨\u{200D}👩\u{200D}👧";
    let source = format!("**{emoji}x**");
    let mut v = make_view(&source);
    v.engine_mut().handle_set_cursor_byte(2);

    let result = v.engine().visual_move(2, MoveDirection::Right, None);

    assert_eq!(result, Some(2 + emoji.len()));
}
```

- [ ] **Step 3: Run tests to verify failure**

Run: `cargo test -p edit-plus-markdown visual_move_right_skips --lib`

Expected: FAIL under current `char_pos + 1` implementation.

- [ ] **Step 4: Implement grapheme step**

In `visual_move()`:

- Left: `grapheme_pos - 1`
- Right: `grapheme_pos + 1`
- LineStart: `0`
- LineEnd: `source_map.source_bytes_by_visual_grapheme.len() - 1`
- Up/Down: use `grapheme_at_x(target_line, x)`

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p edit-plus-markdown visual_move_right_skips --lib
cargo test -p edit-plus-markdown visual_move_right_advances_one_grapheme_in_expanded_span --lib
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/markdown/src/view.rs
git commit -m "fix: move markdown editor cursor by grapheme"
```

## Task 5: 迁移 SelectionState 到 Grapheme 坐标

**Files:**
- Modify: `crates/markdown/src/selection.rs`
- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/app/src/dispatch/editor.rs`
- Test: `crates/markdown/src/selection.rs`
- Test: `crates/markdown/src/view.rs`

**Interfaces:**
- Produces: `ViewPos { flat_line_idx, grapheme_pos }`
- Produces: `SelectionState::selected_text()` slices by grapheme boundary
- Produces: `SelectionState::highlights()` calls `grapheme_x()`

- [ ] **Step 1: Rename `char_pos` to `grapheme_pos`**

Update `ViewPos`:

```rust
pub struct ViewPos {
    pub flat_line_idx: usize,
    pub grapheme_pos: usize,
}
```

Update plugin query responses still using `(usize, usize)` as pure tuple; document that second element is visual grapheme index for Markdown editor/preview.

- [ ] **Step 2: Write failing selection tests**

```rust
#[test]
fn selection_selected_text_does_not_split_combining_grapheme() {
    let flat_lines = vec![FlatLine {
        flat_idx: 0,
        rect: Rect::new(0.0, 0.0, 100.0, 20.0),
        text: "xe\u{0301}y".to_string(),
        font_size: 14.0,
        shaped: None,
        source_bytes_by_visual_grapheme: None,
    }];
    let mut sel = SelectionState::new();
    sel.anchor = Some(ViewPos { flat_line_idx: 0, grapheme_pos: 1 });
    sel.cursor = Some(ViewPos { flat_line_idx: 0, grapheme_pos: 2 });

    assert_eq!(sel.selected_text(&flat_lines), Some("e\u{0301}".to_string()));
}
```

- [ ] **Step 3: Run tests to verify failure**

Run: `cargo test -p edit-plus-markdown selection_selected_text_does_not_split --lib`

Expected: FAIL until selection slicing is grapheme-based.

- [ ] **Step 4: Update preview-mode keyboard selection**

In `crates/app/src/dispatch/editor.rs`, preview non-editing `ExtendLeft/Right/LineEnd/DocEnd` currently uses `fl.text.chars().count()`. Replace with a plugin query if available:

```rust
PluginQuery::FlatLines
```

must eventually expose grapheme counts, or add a pure protocol `FlatLineMetrics { text, grapheme_count }`. If adding a protocol, place it in `crates/ui/src/plugin.rs`.

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p edit-plus-markdown selection_selected_text_does_not_split --lib
cargo test -p edit-plus-markdown --lib -- selection
cargo test -p edit-plus-app --lib -- ExtendRight
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/markdown/src/selection.rs crates/markdown/src/view.rs crates/app/src/dispatch/editor.rs crates/ui/src/plugin.rs
git commit -m "fix: make markdown selection grapheme-aware"
```

## Task 6: Host 同步必须回读 Snapped Cursor Byte

**Files:**
- Modify: `crates/app/src/dispatch/wysiwyg.rs`
- Modify: `crates/app/src/dispatch/mouse.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Test: `crates/app/src/app_tests.rs`
- Test: `crates/app/src/dispatch/editor.rs`

**Interfaces:**
- Produces: helper `set_wysiwyg_cursor_byte_synced(requested_byte: usize) -> Option<usize>` or equivalent private method.
- Consumes: `DocumentView::cursor_move_to_offset()` and `DocumentView::cursor_offset()`.

- [ ] **Step 1: Write failing host sync test**

Use a recording WYSIWYG plugin that returns a mid-cluster byte for `VisualMoveWysiwyg`. The host must notify `SetCursorByte(snapped_byte)`, not the original requested byte.

```rust
#[test]
fn wysiwyg_navigation_notifies_plugin_with_snapped_cursor_byte() {
    let mut app = App::new(None);
    app.push_recording_wysiwyg_doc_for_test("x👨\u{200D}👩\u{200D}👧y");
    app.recording_plugin_set_visual_move_result_for_test(3);

    app.dispatch_edit_command(EditCommand::MoveRight, test_event_loop());

    let doc_byte = app.workspace.active_doc().unwrap().cursor_offset().to_usize();
    let plugin_byte = app.recording_plugin_last_cursor_byte_for_test();
    assert_eq!(plugin_byte, Some(doc_byte));
    assert_ne!(plugin_byte, Some(3));
}
```

- [ ] **Step 2: Run test to verify failure**

Run: `cargo test -p edit-plus-app wysiwyg_navigation_notifies_plugin_with_snapped_cursor_byte --lib`

Expected: FAIL because current code sends `new_byte` directly after `cursor_move_to_offset(new_byte)`.

- [ ] **Step 3: Implement snapped sync helper**

In every WYSIWYG cursor update path:

```rust
tab.doc.cursor_move_to_offset(requested_byte);
let snapped_byte = tab.doc.cursor_offset().to_usize();
tab.plugin.handle_message(PluginMessage::SetCursorByte(snapped_byte), &mut tab.doc);
```

Update:

- `dispatch_wysiwyg_navigation()`
- `wysiwyg_navigate_to_doc_boundary()`
- `wysiwyg_navigate_to_doc_end()`
- `wysiwyg_page()`
- `set_wysiwyg_cursor_from_point()`
- any fallback EOF cursor path

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p edit-plus-app wysiwyg_navigation_notifies_plugin_with_snapped_cursor_byte --lib
cargo test -p edit-plus-app sync_wysiwyg_plugin_state_pushes_source_and_cursor --lib
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/dispatch/wysiwyg.rs crates/app/src/dispatch/mouse.rs crates/app/src/app_renderer.rs crates/app/src/app_tests.rs
git commit -m "fix: sync markdown editor cursor after snapping"
```

## Task 7: 二阶段 Mouse Hit-Test 与 Drag Selection 使用 Grapheme/Snapped Byte

**Files:**
- Modify: `crates/app/src/dispatch/mouse.rs`
- Modify: `crates/markdown/src/view.rs`
- Test: `crates/app/src/dispatch/mouse.rs`
- Test: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: Task 3 `hit_test_byte()` grapheme map
- Consumes: Task 6 snapped byte sync
- Produces: WYSIWYG click/drag invariant: first click, second hit-test, selection cursor all use same snapped source byte.

- [ ] **Step 1: Write failing two-phase combining click test**

```rust
#[test]
fn two_phase_wysiwyg_click_preserves_combining_cursor_byte() {
    let mut app = App::new_with_markdown_editor_for_test("hello **e\u{0301}** here");
    let click = app.test_point_inside_text("e\u{0301}");

    app.dispatch_wysiwyg_click_for_test(click.x, click.y);

    let cursor = app.workspace.active_doc().unwrap().cursor_offset().to_usize();
    assert_eq!(cursor, "hello **".len());
}
```

- [ ] **Step 2: Run test to verify failure**

Run: `cargo test -p edit-plus-app two_phase_wysiwyg_click_preserves_combining_cursor_byte --lib`

Expected: FAIL until click path uses grapheme maps and snapped sync.

- [ ] **Step 3: Update `set_wysiwyg_cursor_from_point()`**

The function must:

1. `sync_wysiwyg_plugin_state()`
2. Phase 1 `HitTestByte`
3. `DocumentView` move and snapped byte回读
4. `SetCursorByte(snapped)`
5. synchronous plugin render
6. Phase 2 `HitTestByte`
7. `DocumentView` move and snapped byte回读
8. `SetCursorByte(final_snapped)`
9. return `final_snapped`

- [ ] **Step 4: Update drag selection**

In WYSIWYG drag branch, call `SetSelCursorByte(Some(final_snapped))`; ensure selection anchor is initialized with the snapped mouse-down byte.

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p edit-plus-app --lib -- wysiwyg
cargo test -p edit-plus-markdown two_phase_hit_test_from_folded_to_expanded_span --lib
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/dispatch/mouse.rs crates/markdown/src/view.rs
git commit -m "fix: stabilize markdown editor mouse cursor mapping"
```

## Task 8: 清理旧 Char 命名和补齐状态文档

**Files:**
- Modify: `crates/markdown/src/edit.rs`
- Modify: `crates/markdown/src/layout/types.rs`
- Modify: `crates/markdown/src/layout/context.rs`
- Modify: `crates/markdown/src/selection.rs`
- Modify: `docs/plans/2026-06-27-mdeditor-grapheme-cursor-state-plan.md`

**Interfaces:**
- Produces: no remaining WYSIWYG editor state named `visual_char`, `char_pos`, `source_bytes_by_visual_char` except compatibility comments explicitly marked as preview legacy.

- [ ] **Step 1: Search old names**

Run:

```bash
rg -n "visual_char|char_pos|source_bytes_by_visual_char|chars\\(\\)\\.count\\(\\)" crates/markdown/src crates/app/src/dispatch -S
```

Expected: only non-editing preview paths or intentionally documented compatibility paths remain.

- [ ] **Step 2: Rename remaining editor-facing names**

Rename:

- `char_offset_from_byte` -> `grapheme_offset_from_byte`
- `byte_from_flat_line_and_visual_char` -> `byte_from_flat_line_and_visual_grapheme`
- `find_flat_and_char_for_byte` -> `find_flat_and_grapheme_for_byte`
- comments saying "visual char" -> "visual grapheme"

- [ ] **Step 3: Update this document's status section**

Add an implementation note listing actual renamed functions and any intentionally retained preview-only char paths.

- [ ] **Step 4: Verify full targeted suite**

Run:

```bash
cargo fmt --all -- --check
cargo test -p edit-plus-core backspace_grapheme_cluster_zwj_emoji
cargo test -p edit-plus-app combining_accent_deleted_with_base
cargo test -p edit-plus-app --lib -- wysiwyg
cargo test -p edit-plus-markdown --lib -- wysiwyg
cargo check -p edit-plus-app
```

Expected: PASS.

- [ ] **Step 5: Major verification**

Run: `./scripts/verify.sh`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/markdown/src crates/app/src docs/plans/2026-06-27-mdeditor-grapheme-cursor-state-plan.md
git commit -m "docs: record markdown editor grapheme cursor state plan"
```

---

## Verification Matrix

| Area | Command | Expected |
|------|---------|----------|
| Core grapheme deletion | `cargo test -p edit-plus-core backspace_grapheme_cluster_zwj_emoji` | PASS |
| App combining deletion | `cargo test -p edit-plus-app combining_accent_deleted_with_base` | PASS |
| Markdown grapheme map | `cargo test -p edit-plus-markdown grapheme_map_ --lib` | PASS |
| Markdown WYSIWYG hit-test | `cargo test -p edit-plus-markdown wysiwyg_hit_test_roundtrips --lib` | PASS |
| Markdown WYSIWYG navigation | `cargo test -p edit-plus-markdown visual_move_right_skips --lib` | PASS |
| Markdown selection | `cargo test -p edit-plus-markdown selection_selected_text_does_not_split --lib` | PASS |
| App WYSIWYG sync | `cargo test -p edit-plus-app wysiwyg_navigation_notifies_plugin_with_snapped_cursor_byte --lib` | PASS |
| App WYSIWYG mouse | `cargo test -p edit-plus-app --lib -- wysiwyg` | PASS |
| Formatting | `cargo fmt --all -- --check` | PASS |
| App compile | `cargo check -p edit-plus-app` | PASS |
| Full verification | `./scripts/verify.sh` | PASS |

## Manual Acceptance

- [ ] 在 Markdown editor 中打开 `**é**`，左右移动光标时不能停在 `e` 和 combining mark 中间。
- [ ] 在 Markdown editor 中打开 `**👨‍👩‍👧**`，左右移动一次跨过整个 emoji cluster。
- [ ] 点击 folded `**world**` 的正文，第一次点击后展开 marker，光标仍在点击对应的源码位置。
- [ ] 点击 folded `**é**` 的正文，展开后光标仍在整个 grapheme 的边界。
- [ ] drag selection 跨过 emoji/combining grapheme 时，高亮不能只覆盖半个 grapheme。
- [ ] 输入字符替换选区后，插入位置必须等于选择结束后的 snapped source byte。
- [ ] 上下移动经过软折行时 sticky x 不因 marker 展开/折叠持续漂移。
- [ ] 光标 blink 隐藏/显示只影响 cursor rect 绘制，不改变 layout 或 hit-test 结果。

## Known Follow-Up After This Plan

- 普通编辑器无 `advance_cache` 路径里仍存在 `cursor_column()` byte-as-column 的历史命名，需要单独计划清理。
- Preview-only selection 目前也使用 `(line, char)` 风格协议；如果 preview copy/highlight 也要严格 grapheme-safe，应另开计划迁移 `PluginResponse::FlatLines` 为带 grapheme metrics 的纯数据结构。
- `SetCursorByte(usize)` 协议可继续保留，但长期可以新增 `CursorSourcePosition { byte, affinity }` 以表达 marker 边界处的左右亲和性。
