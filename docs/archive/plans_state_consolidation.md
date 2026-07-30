# 状态同步与所有权 — 问题分析与重构方案

> 制定日期：2026-06-01
> 范围：`text_buffer.rs`（core）、`document_view/mod.rs`、`app.rs`、`mouse.rs`、`commands.rs`、`cursor_motion.rs`、`line_index.rs`、`viewport.rs`、`wrap_index.rs`
> 与 `plans_viewport_virtual_line_analysis.md` 的关系：那份聚焦"视口/虚拟行/autoscroll"，本份聚焦"状态所有权、双份真相源、API 表面"。两份有依赖：本份阶段 3（WrapIndex 移入 DocumentView）会影响视口 plan 的阶段 1.1 签名；建议先完成视口 plan 阶段 1，再启动本份。

---

## 一、问题清单

### R1 `cursor_offset` 双份真相源（高风险）

`core::TextBuffer.cursor.offset` 是真正的真相源；`DocumentView.cursor_offset` 是它的镜像。

镜像同步规则不一致：

| 路径 | 同步方式 | 文件 |
|------|---------|------|
| `cursor_move_*` 系列（9 个方法） | 调 `tb.cursor_move_*` 后从 `tb.cursor_offset()` 回读 | mod.rs:294-387 |
| `extend_selection_left/right/word_*` | 调 tb 后回读 | mod.rs:560/570/599/607 |
| `extend_selection_up/down` | **混合** — 调 `tb.cursor_move_to_logical` 后回读 | mod.rs:577-591 |
| `extend_selection_to_line_*/doc_*` | **直接写 `dv.cursor_offset = ...`，不调 tb** | mod.rs:615/624/631/637 |
| `select_all` | 直接写 | mod.rs:675 |
| `delete_selection` | 调 tb 后回读 | mod.rs:697 |
| `mouse.rs handle_cursor_moved` (拖拽) | **直接写 `dv.cursor_offset = offset`** | mouse.rs:95 |
| `mouse.rs handle_mouse_input` 三击/双击/Shift-click | **直接写** | mouse.rs:138-150 |

**风险**：6 处旁路 → tb 的 `cursor.logical_pos` / `cursor.visual_pos` 滞后 → `EditCommand::PageUp/Down`（commands.rs:202-213）调 `tb.cursor_visual_pos()` 后会得到陈旧坐标 → page 跳到错位置。

复现条件：mouse drag → 立即 PageDown。

### R2 选区双份真相源（中风险）

`DocumentView.selection_anchor: Option<usize>` 是 app 的真相源；`core::TextBuffer` 内部的 `set_selection / selection_update_offset` 是 tb 的真相源。

99% 时间两者**不同步**：
- app 通过 `selection_anchor` 维护选区
- tb 的 selection 仅在 `delete_selection`（mod.rs:686）瞬间被临时填充，目的只是借用 `extract_user_selection` API
- 调用 `extract_user_selection(true)` 之后 tb 的 selection 与 cursor 状态没有明确文档化

**风险**：
- 任何依赖 `tb.selection` 的 core API 都会读到 stale 数据
- `extract_user_selection` 之后 cached_cursor_line 的失效靠 `cursor_offset` 比对（隐式）而非显式 invalidate

### R3 "光标 doc line" 三份派生

| 来源 | 备注 |
|------|------|
| `DocumentView.cached_cursor_line: Option<(offset, line)>` | app 自维护 |
| `core::TextBuffer.cursor.logical_pos.y` | tb 内部维护 |
| `LineIndex.offsets.partition_point(...)` | 派生函数 |

三者大多时候相等，但 logical_pos 在 R1 的 6 处旁路后会落后。`logical_pos.y` 实际上**没有 reader**（commands.rs 用 `cursor_visual_pos()`，不用 logical），只是没人读所以没暴露问题。

### R4 "行字节偏移/长度"两份

- `DocumentView.line_index`（自己扫换行符）
- `core::TextBuffer` 内部行结构（用于 reflow / `cursor_logical_pos` / `stats.logical_lines`）

两套都对，但**两套都在每次编辑时响应**：tb 内部的 `recalc_after_content_changed` + app 的 `line_index.rescan_from`。重复工作。

### R5 "可视行计算"两套并行

- app 自己用 `compute_visual_lines` + `wrap_cache` 算（基于像素 advance）→ 写入 `WrapIndex`
- core::TextBuffer 也有 `reflow()` / `stats.visual_lines` 路径（基于字符列宽）

两套**并行存在但不交互**。app 完全忽略 core 的可视行结果（注释 text_buffer.rs:614 也明示 core 假设 visual_lines == logical_lines 当无 wrap 时）。

### R6 总行数四份（已在视口 plan P1-2 点出）

`Viewport.total_lines` / `Viewport.total_visual_lines` / `WrapIndex.len()` / `WrapIndex.total_display_rows()` + 第五份 `core::TextBufferStatistics`。

### R7 `LineCache.clusters` vs `AdvanceCacheEntry.clusters` 数据结构重复

- `cursor_motion::LineCache.clusters: Vec<(byte_start, byte_end, advance)>` — 每 cluster 的原始 advance
- `render_geom::AdvanceCacheEntry.clusters: Vec<(byte_end, cumulative_x)>` — 已累加好的 x

两个结构表示同一个东西的不同投影，导致 `move_cursor_visual` 内联版（行 255-269）和 `find_closest_offset`（cursor_motion.rs:52）算法略有差异（细节处理不一致）。

---

## 二、所有权错配

### O1 WrapIndex 持在 App，但所有逻辑在 DocumentView/Viewport 中

```rust
// 当前 9 个方法都带 Option<&WrapIndex> 参数
pub fn cursor_move_left(&mut self, wrap_index: Option<&WrapIndex>);
pub fn cursor_move_right(&mut self, wrap_index: Option<&WrapIndex>);
pub fn cursor_move_to_offset(&mut self, offset: usize, wrap_index: Option<&WrapIndex>);
pub fn cursor_move_word_left(&mut self, wrap_index: Option<&WrapIndex>);
pub fn cursor_move_word_right(&mut self, wrap_index: Option<&WrapIndex>);
pub fn cursor_move_to_line_start(&mut self, wrap_index: Option<&WrapIndex>);
pub fn cursor_move_to_line_end(&mut self, wrap_index: Option<&WrapIndex>);
pub fn cursor_move_up(&mut self, wrap_index: Option<&WrapIndex>);
pub fn cursor_move_down(&mut self, wrap_index: Option<&WrapIndex>);
// 还有 insert_at_cursor / delete_backward / delete_forward / sync_cursor / undo / redo
```

所有调用方几乎都是 `Some(&self.wrap_index)`。`Option` 实质是"DocumentView 不持有 wrap_index"的设计借口。

**架构动机**：一个文档对应一份 WrapIndex；切换文档时 WrapIndex 应跟随。当前 `app.rs:280` 切换文档时 `self.wrap_index = WrapIndex::new(line_count)` 整个换掉——这正说明 WrapIndex 的生命周期与文档绑定。

### O2 first_line / last_line / advance_cache 持在 App，但语义属于"当前帧的渲染产物"

`first_line`、`last_line`、`advance_cache`、`cursor_visual_line`、`cursor_visual_line_in_doc`、`cursor_pixel_x`、`sticky_x` 都是 render 写、其他模块（mouse、cursor_motion）读的"帧间状态"。

它们目前作为 App 的字段，被 9 个 `&mut` 参数传给 `shape_visible_lines`。封装为单一结构（如 `FrameRenderState`）能：
- 一次传一个 `&mut`
- 让"读帧状态"的代码（cursor_motion、mouse hit-test、selection vertices）走单一接口
- 切换文档时整体清零更明确

### O3 `App.first_line_doc_offset` 死字段

`first_line: LineCache` 里已经有 `doc_offset` 字段（render 在写、cursor_motion 在读）。`App.first_line_doc_offset` 是另一个独立顶层字段，**初始化后从未被读写**（仅出现在 declaration 和 initialization）。

---

## 三、冲突点

### F1 mouse 旁路 cursor 协议（已在 R1 列出）

`mouse.rs:95/138-150` 直接写 `dv.cursor_offset = offset`，绕过 `cursor_move_to_offset` → 不触发 `sync_cursor` → 不调 `ensure_cursor_visible_sync`。

当前靠 `pre_shape_autoscroll` 在 render 前救场（因 `cursor_offset != last_cursor_offset` 而触发）。**正确性可疑**：拖拽到屏幕外的 autoscroll 依赖一个本意是"修正帧间延迟"的机制。

### F2 `extend_selection_up/down` 用 logical 不用 visual

`mod.rs:577-591` 通过 `tb.cursor_logical_pos()` + `cursor_move_to_logical(y±1)` 实现。
**logical 是 doc-line 级**，跳过 wrap 内部 visual 行。

而无 Shift 的 `cursor_move_up/down`（mod.rs:374-387）也用 logical（同一个 BUG），但被 `App::move_cursor_visual` 拦截走 visual 路径（`app.rs:648-655`）。Shift+方向键**没有**这层拦截 → 与方向键行为不一致。

复现条件：长行 wrap → Shift+Down → 选区跳过 wrap 内行。

### F3 `wrap_index.set_viewport_width` 后树值与 dirty 标记不一致

`set_viewport_width(w)` 改宽度后调 `mark_all_dirty()`，**树值不重置**（注释明确 intentional）。

之后 `clamp_scroll_top` 用旧的 `total_display_rows`。下一帧 shape 增量更新部分行后 → `total_display_rows` 是"新宽度 N 行 + 旧宽度 M 行"的混合。

resize 瞬间滚动条可能跳一下。

### F4 `delete_selection` 中 cursor 移动语义不明

`mod.rs:684-688`:
```rust
self.tb.cursor_move_to_offset(start);
self.tb.selection_update_offset(end);
self.tb.extract_user_selection(true);
self.cursor_offset = self.tb.cursor_offset();
```

`extract_user_selection(true)` 之后 tb cursor 在哪？依赖 core 实现细节。`cached_cursor_line` 没显式 invalidate，靠 `cursor_offset` 改变隐式 cache miss。脆。

---

## 四、命名 / 表面问题

### N1 `cursor_visual_line` vs `cursor_visual_line_in_doc` 易混淆

| 字段 | 实际语义 | 建议名 |
|------|---------|--------|
| `cursor_visual_line: Option<usize>` | 屏幕第几行（0..visible_rows） | `cursor_screen_row` |
| `cursor_visual_line_in_doc: usize` | 当前 doc line 内第几个 wrap 行 | `cursor_wrap_offset` |

### N2 `Viewport.scroll_top` 单位文档化但 API 单位不齐

文档头注释写 `scroll_top` 是 DisplayRow（fractional），但：
- `scroll_to_doc_line(line)` 把 doc line 当 row 用（视口 plan 已点出）
- `scroll_down/up(delta)` 把 doc-unit delta 当 DisplayRow delta 用（视口 plan 已点出）
- `is_at_bottom` 三单位混用（视口 plan 已点出）

本 plan 不重复处理，由视口 plan 阶段 5 完成。

---

## 五、性能可疑点

### P1 backspace/delete 总走慢路径

`mod.rs:271-290`：`delete_backward / delete_forward` 调 `sync_after_edit_incremental(..., may_have_newline=true, ...)` — **硬编码为 true**。

`sync_after_edit_incremental` fast path 要求 `!may_have_newline`，所以删除路径**永远走 rescan_from**，即使删的是非换行字符。

正确做法：
```rust
pub fn delete_backward(&mut self, count: usize, wrap_index: Option<&WrapIndex>) {
    // 删除前先 peek 即将被删的字节，判断是否含换行
    let cursor = self.cursor_offset;
    let may_have_newline = self.peek_bytes_before(cursor, count_bytes_for(count))
        .iter().any(|&b| b == b'\n' || b == b'\r');
    self.tb.delete(CursorMovement::Grapheme, -(count as isize));
    self.sync_after_edit_incremental(old_len, old_line_count, may_have_newline, wrap_index);
}
```

注意：count 是 grapheme 数，需要先转换为字节范围才能 peek。如果实现复杂可保留现状，仅作为 P3 等长期优化。

---

## 六、死代码清单

视口 plan 已涉及一部分。本 plan 一次性扫尾：

| 项 | 位置 | 状态 |
|----|------|------|
| `App.first_line_doc_offset` | app.rs:100 | 死字段 |
| `LineIndex::line_byte_offset` | line_index.rs:181 | `#[allow(dead_code)]` |
| `LineIndex::line_length` | line_index.rs:187 | `#[allow(dead_code)]` |
| `LineIndex::binary_search_offset` | line_index.rs:193 | `#[allow(dead_code)]` |
| `LineIndex::shift_offsets_after` | line_index.rs:199 | `#[allow(dead_code)]` |
| `LineIndex::shift_line_length` | line_index.rs:208 | `#[allow(dead_code)]` |
| `DocumentView::sync_after_edit_full` | mod.rs:736 | `#[allow(dead_code)]` |
| `cursor_motion::move_in_cache` | cursor_motion.rs:86 | `#[allow(dead_code)]` |
| `core::TextBuffer::make_cursor_visible` / `take_cursor_visibility_request` | text_buffer.rs:509/514 | TODO 注释明示是 TUI 耦合，app 无 reader |
| `app.rs.bak` | crates/app/src/ | 临时备份文件，git 未跟踪 |

---

## 七、修改方案（原子化阶段）

每个阶段独立可编译、可测试、可回滚。前置：视口 plan 阶段 1 完成（这样 cursor_visual_line_in_doc 不再是只写字段，整体的"光标-视口"语义稳定下来再做状态收口）。

### 阶段 1：cursor_offset 收口（R1 + F1）

**目标**：让 `tb.cursor.offset` 成为唯一真相源，`dv.cursor_offset` 永远等于 `tb.cursor_offset()`。
**文件**：`document_view/mod.rs`、`mouse.rs`、`commands.rs` 的旁路点
**风险**：低 — 改的都是同一个类型，行为只增不减。

#### 1.1 在 `DocumentView` 加私有方法 `set_cursor_offset_synced`

```rust
/// 唯一允许写 cursor_offset 的入口。强制走 tb，保证 logical_pos / visual_pos 同步。
fn set_cursor_offset_synced(&mut self, offset: usize) {
    self.tb.cursor_move_to_offset(offset);
    self.cursor_offset = self.tb.cursor_offset();
    self.cached_cursor_line = None;
}
```

#### 1.2 替换 6 处直接赋值

```
mod.rs:615 / 624 / 631 / 637 / 675   ← extend_selection_to_*, select_all
mouse.rs:95 / 139 / 144 / 150        ← drag, triple-click, double-click, shift-click
```

全部改为 `dv.set_cursor_offset_synced(X)`。

#### 1.3 `pub cursor_offset` 字段降为 `pub(crate)` 或 `pub` getter

```rust
// 将字段变为只读（外部）
#[deprecated(note = "use cursor_offset() getter; mutate via cursor_move_*")]
pub cursor_offset: usize,
// 或
pub(crate) cursor_offset: usize,
pub fn cursor_offset(&self) -> usize { self.cursor_offset }
```

强制所有外部修改路径走方法。

**验证**：
- `cargo test -p edit-plus-app --lib` 全绿
- 回归用例：mouse drag 选区后立即 PageDown，page 跳到正确位置（非 drag 前的位置）
- Shift-click 后立即 PageUp 同上

---

### 阶段 2：选区扩展统一走 visual（F2）

**目标**：`extend_selection_up/down` 与 `move_cursor_visual` 行为一致——按 visual 行走，而非 doc line。
**文件**：`document_view/mod.rs`、`app.rs`、`commands.rs`、`cursor_motion.rs`
**风险**：中 — 改动 cursor_motion 的复用面。

#### 2.1 提取 `move_cursor_visual` 的 byte-offset 计算逻辑

当前 `App::move_cursor_visual`（app.rs:317）做两件事：
1. 计算 visual 移动后的目标字节偏移
2. 调 `dv.cursor_move_to_offset` 应用

把 (1) 拆出为 `cursor_motion::compute_visual_target(delta, ctx, dv) -> Option<usize>`（实质就是当前的 `move_cursor_visual` 自由函数，已经是这个签名，只需要让它对 selection 路径友好）。

#### 2.2 `extend_selection_up/down` 改用 visual 路径

```rust
pub fn extend_selection_up(&mut self, ctx: CursorContext) {
    self.ensure_selection_active();
    if let Some(target) = compute_visual_target(-1, ctx, self) {
        self.set_cursor_offset_synced(target);
    }
}
```

注意：`extend_selection_*` 在 mod.rs，但 `CursorContext` 在 cursor_motion.rs。可能需要把签名上移到 app.rs 层（让 app 持有 ctx 后调一个新方法 `dv.extend_selection_to_offset(target)`）。

```rust
// 实际推荐：在 app.rs 加
fn extend_selection_visual(&mut self, delta: isize) {
    let ctx = self.build_cursor_context();
    if let Some(dv) = self.doc_view.as_mut() {
        if let Some(target) = compute_visual_target(delta, ctx, dv) {
            dv.ensure_selection_active();
            dv.set_cursor_offset_synced(target);
        }
    }
}

// 然后 commands.rs 中
EditCommand::ExtendUp => app.extend_selection_visual(-1),
EditCommand::ExtendDown => app.extend_selection_visual(1),
```

#### 2.3 同样处理 `cursor_move_up/down` 的逻辑

`mod.rs:374-387` 当前实现也用 logical，但被 `App::move_cursor_visual` 拦截。可以让 `dv.cursor_move_up/down` **要求 visual 路径**或**直接删除**——目前 commands.rs:80/84 调它们时已经在 visual 路径外，可能是死路径，需要 grep 确认。

**验证**：
- 回归用例：长行 wrap 30 行 → 光标在第 5 个 wrap 行 → Shift+Down → 选区扩展到第 6 个 wrap 行（不是跳到下一 doc line）
- 普通短行 Shift+Down 行为不变
- `cargo test` 全绿

---

### 阶段 3：WrapIndex 移入 DocumentView（O1）

**目标**：消除 9 个方法的 `Option<&WrapIndex>` 参数；WrapIndex 与文档同生命周期。
**文件**：`document_view/mod.rs`、`app.rs`、`commands.rs`、`mouse.rs`、`render_pipeline.rs`
**风险**：中高 — 大面积签名变更，但每处都是机械替换。

依赖：视口 plan 阶段 1（接回 post_shape_update 后 wrap_index 在 render 路径上的契约稳定）。

#### 3.1 把 wrap_index 字段从 App 搬到 DocumentView

```rust
// document_view/mod.rs
pub struct DocumentView {
    pub(crate) tb: TextBuffer,
    pub(crate) line_index: LineIndex,
    pub(crate) wrap_index: WrapIndex,  // ← 新增
    pub viewport: Viewport,
    // ...
}
```

`Viewport::clamp_scroll_top` 等需要 WrapIndex 的方法通过 `self.viewport.clamp_scroll_top(&self.wrap_index)` 调用，从 DocumentView 内部传入。这些已是视口 plan 阶段 4 的工作；本阶段只负责"持有"。

#### 3.2 删除 9 个方法的 `Option<&WrapIndex>` 参数

```rust
// 之前
pub fn cursor_move_left(&mut self, wrap_index: Option<&WrapIndex>);

// 之后
pub fn cursor_move_left(&mut self);
```

#### 3.3 调用方机械替换

`app.rs` 中所有 `dv.cursor_move_*(Some(&self.wrap_index))` → `dv.cursor_move_*()`。`commands.rs` 同。

#### 3.4 渲染路径：暴露 `&mut wrap_index` getter

```rust
// document_view/mod.rs
pub fn wrap_index(&self) -> &WrapIndex { &self.wrap_index }
pub(crate) fn wrap_index_mut(&mut self) -> &mut WrapIndex { &mut self.wrap_index }
```

`render_pipeline::shape_visible_lines` 改签名：少一个 `wrap_index: &mut WrapIndex` 参数，从 `dv` 借出。

#### 3.5 切换文档时 WrapIndex 自然清零

`app.rs:280` 删除独立的 `self.wrap_index = WrapIndex::new(line_count)` —— 由 `DocumentView::from_file` 内部初始化。

**验证**：
- `cargo test` 全绿
- 切换文档（如果有此功能）后 scroll/cursor 行为正常
- 性能不变（WrapIndex 仍在每帧 shape 时使用）

---

### 阶段 4：选区双份真相源 — 显式协议（R2 + F4）

**目标**：明确定义"app 的 selection_anchor 是真相源，tb selection 仅在 delete_selection 中临时使用"。
**文件**：`document_view/mod.rs`
**风险**：低 — 不改逻辑，只加文档/断言。

#### 4.1 在 `DocumentView` 文档头加显式契约

```rust
//! ## Selection state
//!
//! `selection_anchor: Option<usize>` is the single source of truth for selections.
//! `tb`'s internal selection is **not** synchronized — it is only filled briefly
//! inside `delete_selection` to reuse `extract_user_selection`.
//! After any operation that may touch tb's selection, callers must NOT read
//! tb's selection state — always go through `selection_anchor` / `selection_range()`.
```

#### 4.2 `delete_selection` 末尾显式清理 tb selection

```rust
pub fn delete_selection(&mut self) -> bool {
    // ... existing logic ...
    self.tb.extract_user_selection(true);
    self.cursor_offset = self.tb.cursor_offset();
    self.cached_cursor_line = None;  // ← 显式 invalidate，不靠 offset 比对
    self.selection_anchor = None;
    // tb's internal selection is now in an unspecified state — that's OK,
    // we never read it.
}
```

#### 4.3 添加 debug-only 断言：所有 `cursor_offset` 读路径前 `tb.cursor_offset() == self.cursor_offset`

```rust
#[cfg(debug_assertions)]
fn assert_cursor_synced(&self) {
    debug_assert_eq!(
        self.tb.cursor_offset(),
        self.cursor_offset,
        "cursor_offset desynced from tb"
    );
}
```

阶段 1 完成后这条断言应当永远不触发；用作回归保护。

**验证**：
- `cargo test` 全绿（debug_assertions 默认开启）
- 各类编辑/选区/删除操作不触发断言

---

### 阶段 5：清理死代码与命名（Z + N1 + R7）

**目标**：消除冗余结构，统一命名，删除死路径。
**文件**：多处机械删除
**风险**：低 — 仅删除/重命名。

#### 5.1 删除死代码

| 删除对象 | 文件 |
|---------|------|
| `App.first_line_doc_offset` 字段 | app.rs:100 |
| `LineIndex` 5 个 `#[allow(dead_code)]` 方法 | line_index.rs |
| `DocumentView::sync_after_edit_full` | mod.rs:736 |
| `cursor_motion::move_in_cache` | cursor_motion.rs:86 |
| `core::TextBuffer::make_cursor_visible` / `take_cursor_visibility_request` | text_buffer.rs:509/514 — 谨慎，core 是上游库，需先确认无其它消费者 |
| `app.rs.bak` | 删除文件 |

#### 5.2 重命名 cursor 字段

```rust
cursor_visual_line       → cursor_screen_row
cursor_visual_line_in_doc → cursor_wrap_offset
```

更新 render_pipeline、cursor_motion、app 的引用。

#### 5.3 统一 cluster cache 数据结构（R7）

选定 `(byte_start, byte_end, advance)` 为标准（信息更全，可派生出累积 x）：

```rust
// 删除 AdvanceCacheEntry.clusters: Vec<(usize, f32)> 这个累积形式
// 统一为 Vec<(usize, usize, f32)>
// 在 mouse hit-test 和 4a 内联段中现场累加
```

或反方向（选累积 x），但需要把 `LineCache.clusters` 改为同形式 + 现场反推 byte_start。**推荐第一种**，原始 advance 信息无损。

`find_closest_offset`（cursor_motion.rs:52）成为唯一的"按 sticky_x 找字节"实现，删除 `move_cursor_visual` 内联段（行 255-269）。

**验证**：
- `cargo test` 全绿
- mouse hit-test、Up/Down 行为不变
- `cargo build` 无 dead_code 警告

---

### 阶段 6（可选）：FrameRenderState 封装（O2）

**目标**：把每帧产物（first_line/last_line/advance_cache/cursor_*）封装到一个结构。
**文件**：`app.rs`、`render_pipeline.rs`、`cursor_motion.rs`、`mouse.rs`
**风险**：中 — 大量参数列表精简，但行为不变。

```rust
pub(crate) struct FrameRenderState {
    pub first_line: LineCache,
    pub last_line: LineCache,
    pub advance_cache: Vec<AdvanceCacheEntry>,
    pub cursor_screen_row: Option<usize>,
    pub cursor_wrap_offset: usize,
    pub cursor_pixel_x: f32,
    pub sticky_x: f32,
    pub sticky_x_dirty: bool,
}

impl FrameRenderState {
    pub fn clear_for_new_doc(&mut self) { /* ... */ }
}
```

`shape_visible_lines` 签名从 9 个 `&mut` 参数变为 `&mut FrameRenderState`。

**收益**：切换文档时整体清零更明确；测试构造帧状态更直接。

**风险**：相对前 5 个阶段收益较小，可作为后续优化。

---

### 阶段 7（长期）：S1 backspace 慢路径

如阶段 1-6 之外仍有性能需求，再做。涉及"按 grapheme 数 peek 字节"的额外工作，复杂度不低。

---

## 八、阶段依赖关系

```
（视口 plan 阶段 1）  ← 前置
  ↓
阶段 1（cursor_offset 收口）
  ↓
阶段 2（selection visual）— 依赖阶段 1 的 set_cursor_offset_synced
  ↓
阶段 3（WrapIndex 移入 DV）— 依赖视口 plan 阶段 4 的 clamp_scroll_top(wi) 签名稳定
  ↓
阶段 4（selection 协议）— 独立，可并行
  ↓
阶段 5（清理 + 命名）— 最后扫尾
  ↓
阶段 6（FrameRenderState）— 可选
阶段 7（backspace 慢路径）— 长期优化，独立
```

---

## 九、风险与缓解

| 风险 | 缓解 |
|------|------|
| 阶段 1 把 `cursor_offset` 改为 `pub(crate)` 后破坏外部消费者 | 全 crate grep 一次；如 `tests/`、`benches/` 直接读字段，加 getter 后逐一替换 |
| 阶段 2 改 cursor_motion 接口影响视口 plan 测试 | 先在视口 plan 阶段 1 完成后再启动；测试用例保留旧的 ctx 构造方式 |
| 阶段 3 WrapIndex 移入 DV 后切换文档逻辑变化 | 加单测：构造两个 DV，分别有不同 wrap state，确认互不影响 |
| 阶段 5 删除 core 的 `make_cursor_visible` 影响其它 crate | core 是 vendor 进来的；删除前 grep 整个 workspace 确认无消费 |
| 阶段 6 FrameRenderState 与现有测试模式不兼容 | 阶段 6 标为可选，测试显示成本超过收益时跳过 |

---

## 十、回归用例

每个阶段必跑：

| 用例 | 期望 | 覆盖 |
|------|------|------|
| Mouse drag 后立即 PageDown | page 跳到 drag 终点对应位置（非 drag 前） | R1 / F1 |
| Shift-click 后立即 PageUp | 同上 | R1 / F1 |
| 长行 wrap → Shift+Down | 选区扩展到下一个 visual 行（不跳整 doc line） | F2 |
| `delete_selection` 后立即 cursor_line | 返回新位置的 line（不是 cached 的旧值） | F4 |
| 切换文档（如有此功能） | scroll、cursor、selection 全部清零 | O1 |
| 长时间编辑 → debug 断言 | 不触发 cursor desync | 阶段 4 |

---

## 十一、预期收益

| 指标 | 当前 | 修改后 |
|------|------|--------|
| `dv.cursor_offset = X` 旁路点 | 6 处 | 0 |
| 9 个方法的 `Option<&WrapIndex>` 参数 | 9 个 | 0 |
| 死字段 / 死方法 | 9+ 项 | 0 |
| Cluster cache 数据结构种类 | 2 种 | 1 种 |
| 选区行为与方向键一致性 | 不一致（Shift 走 logical） | 一致（都走 visual） |
| 命名歧义 | `cursor_visual_line` vs `_in_doc` 易混 | 改名后语义清晰 |
| Cursor 状态不变量 | 隐式（靠测试覆盖） | 显式（debug 断言） |
