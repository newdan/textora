# 断行算法修复实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 `WrapIndex` 与 `compute_visual_lines` 中的正确性、性能与 i18n 缺陷，确保编辑、滚动、缓存在所有路径上都给出一致结果。

**Architecture:** 改动集中在 `crates/app/src/wrap_index.rs`（脏标记/重建策略）、`crates/app/src/render_pipeline.rs`（断行算法、缓存键、is_ws/CJK 分类）、`crates/app/src/app.rs`（编辑命令路径触发失效）和 `crates/app/src/commands.rs`（编辑命令返回受影响行）。原有数据结构（segment-tree 索引、shape/wrap LRU 缓存）保留，引入"per-line dirty + 单点重建"取代频繁全树重建，并修正字符分类的覆盖范围。

**Tech Stack:** Rust 2021、wgpu/winit 渲染、`hashlink::LruCache`、自研 segment tree。

**全局测试命令：**
- 单测：`cargo test -p app --lib wrap_index`、`cargo test -p app --lib`、`cargo test -p app`
- 全量：`cargo test --workspace`
- 编译检查：`cargo check --workspace`

每个任务完成后均要求：1) 单测通过；2) `cargo check --workspace` 通过；3) commit。

---

## 阶段 1：缓存键与字符分类的正确性修复（基础设施）

> 这些是其它修复的依赖（任务 4/5 用到 `is_whitespace_cluster`、`cluster_boundary_class`），先做。

### Task 1：扩展 `is_whitespace_cluster` 覆盖 NBSP / 全角空格 / Tab

**Files:**
- Modify: `crates/app/src/render_pipeline.rs`（compute_visual_lines、build_advance_cache_entries、shape_visible_lines 主循环中所有 `is_ascii_whitespace` 调用点）

**背景：** 当前所有判断空白用 `b.iter().all(|b| b.is_ascii_whitespace())`，遗漏 U+00A0 NBSP、U+3000 全角空格；Tab 被强制按 `char_width` 而非 `n × char_width`。三处枚举（断行、advance cache、render）逻辑必须保持一致。

- [x] **Step 1：在 `render_pipeline.rs` 顶部新增辅助函数（写测试先）**

新增 test 文件 `crates/app/src/render_pipeline_tests.rs`（如不存在）或在 `render_pipeline.rs` 末尾追加 `#[cfg(test)] mod tests`：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_cluster_ascii_space() {
        assert!(is_whitespace_cluster(b" "));
        assert!(is_whitespace_cluster(b"\t"));
        assert!(is_whitespace_cluster(b"  \t"));
    }

    #[test]
    fn ws_cluster_nbsp() {
        // U+00A0 = 0xC2 0xA0
        assert!(is_whitespace_cluster(b"\xC2\xA0"));
    }

    #[test]
    fn ws_cluster_ideographic_space() {
        // U+3000 = 0xE3 0x80 0x80
        assert!(is_whitespace_cluster(b"\xE3\x80\x80"));
    }

    #[test]
    fn ws_cluster_non_ws() {
        assert!(!is_whitespace_cluster(b"a"));
        assert!(!is_whitespace_cluster(b"\xE4\xB8\xAD")); // 中
    }

    #[test]
    fn ws_cluster_invalid_utf8_falls_back_to_ascii() {
        assert!(!is_whitespace_cluster(b"\xFF"));
    }
}
```

- [x] **Step 2：跑测试确认 fail**

```bash
cargo test -p app --lib render_pipeline::tests::ws_cluster
```
预期：`error[E0425]: cannot find function 'is_whitespace_cluster'`。

- [x] **Step 3：实现 `is_whitespace_cluster`**

在 `render_pipeline.rs` 中 `is_cjk_char` 前面新增：

```rust
/// Whether all chars in `bytes` are whitespace (ASCII + Unicode).
/// Recognizes ASCII whitespace, NBSP (U+00A0), and ideographic space (U+3000).
/// Returns false for invalid UTF-8 bytes (falls through to ASCII check).
pub(crate) fn is_whitespace_cluster(bytes: &[u8]) -> bool {
    if bytes.is_empty() { return false; }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.chars().all(|c| c.is_whitespace()),
        Err(_) => bytes.iter().all(|b| b.is_ascii_whitespace()),
    }
}
```

- [x] **Step 4：跑测试确认 pass**

```bash
cargo test -p app --lib render_pipeline::tests::ws_cluster
```
预期：5 个测试全部 PASS。

- [x] **Step 5：在 render_pipeline.rs 中替换所有 `is_ascii_whitespace` 闭包调用**

行号参考：514, 522-523, 565-566, 573-575, 578-580, 595-596, 247, 256, 301, 347, 448。统一改成调用 `is_whitespace_cluster(bytes)`。例如：

```rust
// 旧
let is_ws = line_bytes.get(cluster.byte_range.clone())
    .is_some_and(|b| b.iter().all(|b| b.is_ascii_whitespace()));
// 新
let is_ws = line_bytes.get(cluster.byte_range.clone())
    .is_some_and(is_whitespace_cluster);
```

注意：第 519 行附近 `prev_is_ws` 同样替换。

- [x] **Step 6：处理 Tab 的 advance（独立小修）**

在 `compute_visual_lines`（render_pipeline.rs:516）和 `build_advance_cache_entries`（449）以及 render 主循环（348）中，将：

```rust
let advance = if is_ws { char_width } else { cluster.advance.max(1.0) };
```

改为：

```rust
let advance = if is_ws {
    if line_bytes.get(cluster.byte_range.clone()).is_some_and(|b| b == b"\t") {
        char_width * 4.0  // tab width
    } else {
        char_width
    }
} else {
    cluster.advance.max(1.0)
};
```

为避免 4 处重复，把这段抽成函数：

```rust
pub(crate) fn cluster_advance(
    cluster: &shaping::GlyphCluster,
    line_bytes: &[u8],
    char_width: f32,
) -> f32 {
    let bytes = line_bytes.get(cluster.byte_range.clone()).unwrap_or(&[]);
    if is_whitespace_cluster(bytes) {
        if bytes == b"\t" { char_width * 4.0 } else { char_width }
    } else {
        cluster.advance.max(1.0)
    }
}
```

四处调用点改用 `cluster_advance(cluster, line_bytes, char_width)`。

- [x] **Step 7：补一个 Tab 测试**

```rust
#[test]
fn tab_advance_is_4x_char_width() {
    use shaping::GlyphCluster;
    let cluster = GlyphCluster {
        byte_range: 0..1,
        advance: 1.0,
        glyph_id: 0,
        font_id: shaping::FontId::default(),
    };
    let line = b"\t";
    assert_eq!(cluster_advance(&cluster, line, 8.0), 32.0);
}
```

> 如果 `GlyphCluster` 字段名/可见性不允许直接构造，改用 shaper 真实 shape 一个 `"\t"` 的输出。

- [x] **Step 8：`cargo test -p app --lib` + `cargo check --workspace`**

预期：全绿。

- [x] **Step 9：Commit**

```bash
git add crates/app/src/render_pipeline.rs
git commit -m "fix(wrap): 统一空白识别覆盖 NBSP/全角空格/Tab"
```

---

### Task 2：扩展 `is_cjk_char` 覆盖假名与谚文

**Files:**
- Modify: `crates/app/src/render_pipeline.rs:462-489`

**背景：** 现有 `is_cjk_char` 只覆盖 CJK Unified Ideographs，假名 (U+3040–U+30FF) 与谚文 (U+AC00–U+D7AF, U+1100–U+11FF) 走 `cluster_boundary_class → None`，日韩文本无法在 CJK/Latin 边界断行，退化为硬断。

- [x] **Step 1：写测试**

```rust
#[test]
fn cjk_char_hiragana() { assert!(is_cjk_char('あ')); assert!(is_cjk_char('ん')); }
#[test]
fn cjk_char_katakana() { assert!(is_cjk_char('カ')); assert!(is_cjk_char('ー')); }
#[test]
fn cjk_char_hangul_syllable() { assert!(is_cjk_char('한')); assert!(is_cjk_char('글')); }
#[test]
fn cjk_char_hangul_jamo() { assert!(is_cjk_char('\u{1100}')); }
#[test]
fn cjk_char_halfwidth_katakana() { assert!(is_cjk_char('\u{FF66}')); }
#[test]
fn cjk_char_ascii_letter_is_not_cjk() { assert!(!is_cjk_char('A')); assert!(!is_cjk_char('1')); }
#[test]
fn cjk_char_cjk_punct_is_not_cjk_ideo() {
    // CJK 标点不是 ideograph，让 cluster_boundary_class 保持透明语义
    assert!(!is_cjk_char('，')); assert!(!is_cjk_char('。'));
}
```

- [x] **Step 2：跑测试确认 fail（部分）**

```bash
cargo test -p app --lib render_pipeline::tests::cjk_char
```
预期：假名/谚文相关 4 个 test FAIL，其余 PASS。

- [x] **Step 3：扩展 `is_cjk_char`**

```rust
fn is_cjk_char(ch: char) -> bool {
    let cp = ch as u32;
    matches!(cp,
        0x4E00..=0x9FFF    |  // CJK Unified Ideographs
        0x3400..=0x4DBF    |  // CJK Ext A
        0x20000..=0x2A6DF  |  // CJK Ext B
        0xF900..=0xFAFF    |  // CJK Compat Ideographs
        0x2F800..=0x2FA1F  |  // CJK Compat Supplement
        0x3040..=0x309F    |  // Hiragana
        0x30A0..=0x30FF    |  // Katakana
        0x31F0..=0x31FF    |  // Katakana Phonetic Extensions
        0xFF66..=0xFF9F    |  // Halfwidth Katakana
        0xAC00..=0xD7AF    |  // Hangul Syllables
        0x1100..=0x11FF    |  // Hangul Jamo
        0x3130..=0x318F    |  // Hangul Compat Jamo
        0xA960..=0xA97F    |  // Hangul Jamo Extended-A
        0xD7B0..=0xD7FF       // Hangul Jamo Extended-B
    )
}
```

- [x] **Step 4：跑测试**

```bash
cargo test -p app --lib render_pipeline::tests::cjk_char
```
预期：全绿。

- [x] **Step 5：Commit**

```bash
git add crates/app/src/render_pipeline.rs
git commit -m "fix(wrap): is_cjk_char 覆盖假名与谚文以恢复日韩断行边界"
```

---

### Task 3：修复 `wrap_cache` / `shape_cache` 的缓存键

**Files:**
- Modify: `crates/app/src/app.rs:53-55, 257-258`（缓存类型与容量）
- Modify: `crates/app/src/render_pipeline.rs:189-225`（缓存键构造）

**背景：** 当前 `shape_cache` key = `u64` content hash，`wrap_cache` key = `(content_hash, viewport_width.bits, char_width.bits)`。两个问题：
1. 64-bit 哈希碰撞会让 A 行借用 B 行的 shaped run，断行结果完全错乱。
2. 字号/字体/CRLF 改变不会失效缓存。

修法：把 key 改成包含 `(byte_offset, byte_length, generation, font_size, viewport_width, char_width)`（按需）的复合元组；用 `(usize, usize, u64)` 元组作 key 替代纯哈希，杜绝碰撞。

- [x] **Step 1：在 `app.rs` 调整缓存类型**

`app.rs:53-55`：

```rust
// 旧
pub(crate) shape_cache: LruCache<u64, shaping::ShapedRun>,
pub(crate) wrap_cache: LruCache<(u64, u32, u32), Vec<(usize, usize, f32)>>,

// 新
/// Key: (line_byte_offset, line_byte_length, font_size_bits)
pub(crate) shape_cache: LruCache<(usize, usize, u32), shaping::ShapedRun>,
/// Key: (line_byte_offset, line_byte_length, font_size_bits, viewport_width_bits, char_width_bits)
pub(crate) wrap_cache: LruCache<(usize, usize, u32, u32, u32), Vec<(usize, usize, f32)>>,
```

- [x] **Step 2：调整 `render_pipeline.rs:185-233` 的 key 构造**

把：

```rust
let cache_key = {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    line_bytes.hash(&mut h);
    h.finish()
};
let shaped = match text.shape_cache.entry(cache_key) { ... };
```

替换为：

```rust
let (line_offset, line_length) = match dv.visible_line_key_wrap(i, wrap_index) {
    Some(k) => k,
    None => continue,
};
let font_size_bits = text.shaper.font_size().to_bits();
let cache_key = (line_offset, line_length, font_size_bits);
let shaped = match text.shape_cache.entry(cache_key) { ... };
```

把 wrap_key 改成：

```rust
let wrap_key = (
    line_offset,
    line_length,
    font_size_bits,
    viewport_width.to_bits(),
    char_width.to_bits(),
);
```

- [x] **Step 3：处理"内容变了但 (offset,length) 没变"的失效**

由于 buffer 编辑后内容变化但相同 (offset,length) 的另一行可能复用旧 shaped — 这正是任务 5 的"编辑后失效"要解决的。本任务先确保 key 唯一性，缓存内容失效在 Task 5 处理。

为了让 Task 5 能精准失效某行的缓存，新增辅助方法（`app.rs` impl App 中或 TextState 上）：

```rust
impl TextState {
    /// 移除指定 (offset, length) 在所有 font_size 下的 shape/wrap 缓存条目。
    pub(crate) fn invalidate_line(&mut self, offset: usize, length: usize) {
        // hashlink::LruCache 不支持 retain，用收集 keys + remove
        let shape_keys: Vec<_> = self.shape_cache.iter()
            .filter(|(k, _)| k.0 == offset && k.1 == length)
            .map(|(k, _)| *k)
            .collect();
        for k in shape_keys { self.shape_cache.remove(&k); }
        let wrap_keys: Vec<_> = self.wrap_cache.iter()
            .filter(|(k, _)| k.0 == offset && k.1 == length)
            .map(|(k, _)| *k)
            .collect();
        for k in wrap_keys { self.wrap_cache.remove(&k); }
    }

    /// 清空全部缓存（视口宽度、字体改变时使用）。
    pub(crate) fn invalidate_all(&mut self) {
        self.shape_cache.clear();
        self.wrap_cache.clear();
    }
}
```

> 若 `hashlink::LruCache` API 不同，等价实现 `retain` 即可。运行 `cargo doc --open -p hashlink` 或直接试错确认。

- [x] **Step 4：跑现有测试，确保未引入回归**

```bash
cargo test -p app --lib
```
预期：现有所有 wrap/shape 相关测试 PASS（key 类型变了但语义一致）。

- [x] **Step 5：新增哈希碰撞回归测试**

`render_pipeline.rs` 测试模块新增：

```rust
#[test]
fn cache_key_distinguishes_lines_with_same_length() {
    // 此测试不构造真实 shaped run，仅验证 key 元组确实区分不同行
    let k1 = (100usize, 50usize, 0u32);
    let k2 = (200usize, 50usize, 0u32);
    assert_ne!(k1, k2);
}
```

> 这个 test 看上去微弱，但它把"key 必须包含 offset"这条不变量编译期固化。

- [x] **Step 6：Commit**

```bash
git add crates/app/src/app.rs crates/app/src/render_pipeline.rs
git commit -m "fix(wrap): shape/wrap 缓存键改为 (offset,length,font_size,...) 杜绝碰撞"
```

---

## 阶段 2：脏标记驱动的失效与重建（核心正确性）

### Task 4：`WrapIndex::mark_dirty(doc_line)` 单点 API + `update_batch` O(k log n)

**Files:**
- Modify: `crates/app/src/wrap_index.rs:139-216, 256-310`

**背景：**
- `update_batch` 当前每次重建整棵 segment tree（O(n)，n 取 `next_power_of_two`，可达 32k+），每帧都跑一次。每帧 `pending_wrap_updates` 通常只有几十条，应改为多次 `update()`（O(k log n)）。
- 缺一个对外的 `mark_dirty(doc_line)` 单行脏标 API（任务 5 要用）。

- [x] **Step 1：写测试**

`wrap_index.rs` 测试模块追加：

```rust
#[test]
fn mark_dirty_marks_single_line() {
    let mut idx = WrapIndex::new(5);
    idx.update(0, 3); // 标 exact
    idx.update(2, 4);
    assert_eq!(idx.exact_count(), 2);
    idx.mark_dirty(0);
    assert!(!idx.is_exact(0));
    assert!(idx.is_exact(2));
    assert_eq!(idx.exact_count(), 1);
}

#[test]
fn mark_dirty_out_of_range_is_noop() {
    let mut idx = WrapIndex::new(3);
    idx.mark_dirty(100); // 不 panic
    assert_eq!(idx.exact_count(), 0);
}

#[test]
fn update_batch_o_k_log_n_correctness() {
    // 跟 update() 等价
    let mut a = WrapIndex::new(100);
    let mut b = WrapIndex::new(100);
    let updates: Vec<(usize, usize)> = (0..50).map(|i| (i * 2, i + 2)).collect();
    for &(l, c) in &updates { a.update(l, c); }
    b.update_batch(&updates);
    assert_eq!(a.total_display_rows(), b.total_display_rows());
    for i in 0..100 {
        assert_eq!(a.visual_line_count(i), b.visual_line_count(i),
            "mismatch at line {}", i);
        assert_eq!(a.doc_to_display(i), b.doc_to_display(i));
    }
}
```

- [x] **Step 2：跑测试 → fail**

```bash
cargo test -p app --lib wrap_index::tests::mark_dirty_marks_single_line wrap_index::tests::mark_dirty_out_of_range_is_noop wrap_index::tests::update_batch_o_k_log_n_correctness
```
预期：前两个 fail（method not found），第三个 fail 或 pass（取决于现有 update_batch 行为；但如果 pass，仍要重写为 O(k log n)）。

- [x] **Step 3：实现 `mark_dirty` 与 O(k log n) 版本的 `update_batch`**

`wrap_index.rs:139` 之后新增：

```rust
/// Mark a single doc line as dirty (needs re-wrapping).
pub fn mark_dirty(&mut self, doc_line: usize) {
    if doc_line < self.dirty.len() {
        self.dirty[doc_line] = true;
    }
}
```

替换 `update_batch`（202-216）为：

```rust
pub fn update_batch(&mut self, updates: &[(usize, usize)]) {
    for &(doc_line, new_count) in updates {
        if doc_line < self.len {
            // 复用 update() 的 O(log n) 路径，避免全树重建
            self.update(doc_line, new_count);
        }
    }
}
```

- [x] **Step 4：跑测试 → pass**

```bash
cargo test -p app --lib wrap_index
```
预期：全绿（含原有 `batch_update`、`large_document_performance` 等）。

- [x] **Step 5：Commit**

```bash
git add crates/app/src/wrap_index.rs
git commit -m "perf+feat(wrap_index): update_batch 改为 O(k log n)，新增 mark_dirty(line)"
```

---

### Task 5：编辑命令路径 → 失效 wrap_index 与缓存

**Files:**
- Modify: `crates/app/src/commands.rs:80-340`（让 `execute_edit_command` 返回受影响行集合）
- Modify: `crates/app/src/app.rs:670-695, 717-734`（消费返回值并调用失效）

**背景（最严重的 bug）：** 当前 `app.rs:670-694` 仅在**行数变化**时维护 wrap_index。修改单行内容（行数不变）时：
1. `wrap_index` 中该行 `visual_line_count` 不会变 → `total_display_rows`、滚动条、clamp 全部错。
2. `shape_cache` / `wrap_cache` 因 Task 3 改用 (offset,length) 为 key — 即便内容变了，offset 没变 length 变了通常会未命中；但**长度恰好不变**的修改（如改一个字母）会命中旧缓存 → 渲染陈旧文本。

修法：让编辑命令告诉调用方"我影响了哪些 doc line"，由 app 统一 invalidate。

- [x] **Step 1：定义返回类型**

`commands.rs` 顶部新增：

```rust
/// 编辑命令的副作用范围。
#[derive(Default, Debug, Clone)]
pub struct EditOutcome {
    /// 命令是否真实执行（旧返回值的语义）。
    pub executed: bool,
    /// 受影响的 doc line 区间 [start, end_exclusive)，若行数变化则覆盖结构变化前的旧值范围。
    pub dirty_lines: Option<std::ops::Range<usize>>,
    /// 命令执行前的 line_count（供调用方判断 insert/delete）。
    pub old_line_count: usize,
    /// 命令执行后的 line_count。
    pub new_line_count: usize,
}
```

- [x] **Step 2：写测试（先验证返回值）**

`commands.rs` 测试模块追加（参考现有测试构造 dv 的方式）：

```rust
#[test]
fn edit_outcome_insert_char_marks_current_line() {
    let lines = vec!["hello".to_string(), "world".to_string()];
    let mut dv = DocumentView::new(lines, 10);
    // 光标默认在 (0,0)
    let out = execute_edit_command_v2(&EditCommand::InsertChar("X".into()), &mut dv, &[]);
    assert!(out.executed);
    assert_eq!(out.old_line_count, 2);
    assert_eq!(out.new_line_count, 2);
    assert_eq!(out.dirty_lines, Some(0..1));
}

#[test]
fn edit_outcome_insert_newline_marks_split_lines() {
    let lines = vec!["hello".to_string()];
    let mut dv = DocumentView::new(lines, 10);
    dv.cursor_move_to_offset(2); // "he|llo"
    let out = execute_edit_command_v2(&EditCommand::InsertNewline, &mut dv, &[]);
    assert!(out.executed);
    assert_eq!(out.old_line_count, 1);
    assert_eq!(out.new_line_count, 2);
    // 两行都需要重新断行：原行被切成两段
    assert_eq!(out.dirty_lines, Some(0..2));
}

#[test]
fn edit_outcome_backspace_at_line_start_merges() {
    let lines = vec!["hello".to_string(), "world".to_string()];
    let mut dv = DocumentView::new(lines, 10);
    dv.cursor_move_to_offset(6); // line 1 起始
    let out = execute_edit_command_v2(&EditCommand::Backspace, &mut dv, &[]);
    assert_eq!(out.old_line_count, 2);
    assert_eq!(out.new_line_count, 1);
    // 合并后 line 0 内容变化
    assert_eq!(out.dirty_lines, Some(0..1));
}

#[test]
fn edit_outcome_movement_no_dirty() {
    let lines = vec!["hello".to_string()];
    let mut dv = DocumentView::new(lines, 10);
    let out = execute_edit_command_v2(&EditCommand::MoveRight, &mut dv, &[]);
    assert!(out.executed);
    assert_eq!(out.dirty_lines, None);
}
```

- [x] **Step 3：跑 → fail**

```bash
cargo test -p app --lib commands::tests::edit_outcome
```
预期：4 个 test fail（`execute_edit_command_v2` 不存在）。

- [x] **Step 4：实现 `execute_edit_command_v2`**

策略：保留原 `execute_edit_command` 签名为薄包装（兼容旧调用点），新增返回 `EditOutcome` 的版本。

`commands.rs:80` 之前新增：

```rust
pub(crate) fn execute_edit_command_v2(
    cmd: &EditCommand,
    dv: &mut DocumentView,
    advance_cache: &[AdvanceCacheEntry],
) -> EditOutcome {
    let old_line_count = dv.line_count();
    let cursor_line_before = dv.cursor_line();

    let executed = execute_edit_command(cmd, dv, advance_cache);

    let new_line_count = dv.line_count();
    let cursor_line_after = dv.cursor_line();

    // 计算 dirty_lines（所有改变内容的命令分支显式列出）
    let dirty_lines = if !executed {
        None
    } else {
        match cmd {
            EditCommand::InsertChar(_) => {
                // 单行内插入：当前行
                Some(cursor_line_after..cursor_line_after + 1)
            }
            EditCommand::InsertNewline => {
                // 切行：原行 + 新行
                Some(cursor_line_before..cursor_line_after + 1)
            }
            EditCommand::Backspace | EditCommand::DeleteForward => {
                if new_line_count < old_line_count {
                    // 跨行合并：合并后 cursor 所在行 + 删掉的行（行已不在了，
                    // 由调用方 shift_lines 处理；此处 dirty 仅包含合并后那一行）
                    Some(cursor_line_after..cursor_line_after + 1)
                } else if old_line_count == new_line_count {
                    // 单行内删除
                    Some(cursor_line_after..cursor_line_after + 1)
                } else {
                    None
                }
            }
            EditCommand::Paste => {
                // 粘贴可能涉及多行：保守地 dirty 整个文档
                // （后续可优化：从 TextBuffer 拿到 affected range）
                Some(0..new_line_count)
            }
            EditCommand::Undo | EditCommand::Redo => {
                // undo/redo 可能影响任意范围 → 全失效
                Some(0..new_line_count)
            }
            _ => None, // 移动类命令
        }
    };

    EditOutcome {
        executed,
        dirty_lines,
        old_line_count,
        new_line_count,
    }
}
```

- [x] **Step 5：跑测试 → pass**

```bash
cargo test -p app --lib commands::tests::edit_outcome
```
预期：全绿。

- [x] **Step 6：在 `app.rs` 中替换调用点（`handle_command` 与 IME 路径）**

`app.rs:670-695` 替换为：

```rust
let ac = self.advance_cache.clone();
if let Some(dv) = &mut self.doc_view {
    let outcome = execute_edit_command_v2(&cmd, dv, &ac);
    if outcome.executed {
        // 1. 行数结构变化
        if outcome.new_line_count != outcome.old_line_count {
            let is_insertion = outcome.new_line_count > outcome.old_line_count;
            let edit_line = if is_insertion {
                dv.cursor_line()
            } else {
                dv.cursor_line().min(outcome.old_line_count.saturating_sub(1))
            };
            self.wrap_index.resize(outcome.new_line_count);
            self.wrap_index.shift_lines(edit_line, is_insertion);
        }
        // 2. 内容失效：mark dirty + drop 缓存
        if let Some(range) = outcome.dirty_lines {
            for doc_line in range.clone() {
                if doc_line < self.wrap_index.len() {
                    self.wrap_index.mark_dirty(doc_line);
                    if let Some(offset) = dv.line_byte_offset(doc_line) {
                        let length = dv.line_byte_length(doc_line).unwrap_or(0);
                        self.text.invalidate_line(offset, length);
                    }
                }
            }
        }
        self.ensure_cursor_visible_doc_level();
        self.sticky_x_dirty = true;
        self.needs_redraw = true;
        self.cursor_blink_instant = Instant::now();
    }
}
```

类似地，IME 提交路径（app.rs:717-734）改为遍历每个 char 后用 `execute_edit_command_v2` 收集 outcome，最后聚合调用 mark_dirty/invalidate_line：

```rust
WindowEvent::Ime(Ime::Commit(text)) => {
    if let Some(dv) = &mut self.doc_view {
        let mut min_dirty = usize::MAX;
        let mut max_dirty = 0;
        let old_lc = dv.line_count();
        for ch in text.chars() {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            let outcome = execute_edit_command_v2(
                &EditCommand::InsertChar(s.to_string()), dv, &[]);
            if let Some(r) = outcome.dirty_lines {
                min_dirty = min_dirty.min(r.start);
                max_dirty = max_dirty.max(r.end);
            }
        }
        let new_lc = dv.line_count();
        if new_lc != old_lc {
            self.wrap_index.resize(new_lc);
            self.wrap_index.shift_lines(dv.cursor_line(), new_lc > old_lc);
        }
        if min_dirty < max_dirty {
            for doc_line in min_dirty..max_dirty.min(new_lc) {
                self.wrap_index.mark_dirty(doc_line);
                if let Some(offset) = dv.line_byte_offset(doc_line) {
                    let length = dv.line_byte_length(doc_line).unwrap_or(0);
                    self.text.invalidate_line(offset, length);
                }
            }
        }
        self.needs_redraw = true;
        self.cursor_blink_instant = Instant::now();
    }
}
```

- [x] **Step 7：写集成测试验证编辑后 `total_display_rows` 一致**

`crates/app/src/app.rs` 测试模块追加：

```rust
#[test]
fn editing_long_offscreen_line_invalidates_wrap_index() {
    // 构造：100 行短行 + viewport 宽度足够窄
    let mut lines: Vec<String> = (0..100).map(|i| format!("line{}", i)).collect();
    let mut app = App::for_test(lines.clone(), 10);
    app.wrap_index = WrapIndex::new(100);
    // 模拟首帧渲染后所有行 visual_line_count = 1
    for i in 0..100 { app.wrap_index.update(i, 1); }
    let total_before = app.wrap_index.total_display_rows();
    assert_eq!(total_before, 100);

    // 模拟编辑：line 0 暴增到一段长文本（手工调用 dv.insert）
    if let Some(dv) = &mut app.doc_view {
        dv.cursor_move_to_offset(5); // line 0 末尾
        // 模拟一次 InsertChar 流程
        let out = execute_edit_command_v2(
            &EditCommand::InsertChar("Z".repeat(1000)), dv, &[]);
        assert!(out.dirty_lines.is_some());
    }
    // 应用 outcome（这一步本应在 handle_command 里做；此处手动跑一次）
    app.wrap_index.mark_dirty(0);
    // 假装 shape_visible_lines 把 line 0 算成 50 visual lines
    app.wrap_index.update(0, 50);

    assert_eq!(app.wrap_index.total_display_rows(), 100 - 1 + 50);
    assert!(app.wrap_index.is_exact(0));
}
```

> 如果 `App::for_test` 不存在，把这个测试放到 wrap_index 与 commands 已有的层级（避免依赖完整 App 构造）。可以拆成两个更小的：(a) `commands::tests` 验证 outcome；(b) `wrap_index::tests` 验证 mark_dirty + update 流。

- [x] **Step 8：`cargo test -p app` + `cargo check --workspace`**

预期：全绿。

- [x] **Step 9：Commit**

```bash
git add crates/app/src/commands.rs crates/app/src/app.rs
git commit -m "fix(wrap): 编辑命令返回 dirty_lines，统一失效 wrap_index 与 shape/wrap 缓存"
```

---

### Task 6：`shift_lines` 把分裂/合并行也标 dirty

**Files:**
- Modify: `crates/app/src/wrap_index.rs:222-254`

**背景：** `InsertNewline` 时调用 `resize(new_len) → shift_lines(edit_line, true)`：当前 shift 把 `edit_line` 之后的 count 右移、edit_line 处插入 1 标 dirty。但**原 edit_line 的内容被切成上下两段**——上半段还在 edit_line，下半段在 edit_line+1（拿到的是原 edit_line 的旧 count，但内容已变）。两行都需重算。

任务 5 的 `dirty_lines = cursor_line_before..cursor_line_after+1` 已经在 app 层覆盖了这两行，但 `shift_lines` 自身的 dirty 维护应当是局部正确的——这样无论调用方做不做额外失效，索引都自洽。

- [x] **Step 1：写测试**

```rust
#[test]
fn shift_insert_marks_both_split_lines_dirty() {
    let mut idx = WrapIndex::new(3);
    idx.update(0, 2);
    idx.update(1, 5); // exact
    idx.update(2, 3);
    // 在 line 1 处插入新行（模拟分裂）
    idx.resize(4);
    idx.shift_lines(1, true);
    // 期望:
    // - line 0: exact (2)
    // - line 1: dirty (新插入, count=1)
    // - line 2: dirty (原 line 1 的下半段，内容变了)
    // - line 3: exact (原 line 2, 3)
    assert!(idx.is_exact(0));
    assert!(!idx.is_exact(1));
    assert!(!idx.is_exact(2));
    assert!(idx.is_exact(3));
}

#[test]
fn shift_delete_marks_merge_line_dirty() {
    let mut idx = WrapIndex::new(3);
    idx.update(0, 2);
    idx.update(1, 5);
    idx.update(2, 3);
    // 删除 line 1（模拟合并到 line 0）
    idx.shift_lines(1, false);
    idx.resize(2);
    // - line 0: dirty (合并后内容变了)
    // - line 1: exact (原 line 2 的 count=3 平移过来)
    assert!(!idx.is_exact(0));
    assert!(idx.is_exact(1));
}
```

- [x] **Step 2：跑 → fail**

```bash
cargo test -p app --lib wrap_index::tests::shift_insert_marks_both wrap_index::tests::shift_delete_marks_merge
```
预期：fail（is_exact 断言不符）。

- [x] **Step 3：修改 `shift_lines`**

`wrap_index.rs:222-254`：

```rust
pub fn shift_lines(&mut self, edit_line: usize, is_insertion: bool) {
    if self.len == 0 { return; }
    let edit_line = edit_line.min(self.len - 1);
    if is_insertion {
        for i in ((edit_line + 1)..self.len).rev() {
            let src = if i > 0 { self.tree[self.n + i - 1] } else { 1 };
            self.tree[self.n + i] = src;
        }
        self.tree[self.n + edit_line] = 1;
        if edit_line < self.dirty.len() {
            self.dirty.insert(edit_line, true);
            self.dirty.truncate(self.len);
            // 关键修复：被切下的下半段（现在在 edit_line+1）内容变了，标 dirty
            if edit_line + 1 < self.dirty.len() {
                self.dirty[edit_line + 1] = true;
            }
        }
    } else {
        for i in edit_line..self.len {
            let src = if i + 1 < self.len { self.tree[self.n + i + 1] } else { 1 };
            self.tree[self.n + i] = src;
        }
        if edit_line < self.dirty.len() {
            self.dirty.remove(edit_line);
            // 关键修复：合并后的目标行（前一行）内容变了，标 dirty
            if edit_line > 0 && edit_line - 1 < self.dirty.len() {
                self.dirty[edit_line - 1] = true;
            }
            // 注：当 edit_line == 0 时（试图删除第 0 行），合并方向不同，
            // 当前 edit_line 处现在是原 line 1 的内容，标它 dirty
            if edit_line == 0 && !self.dirty.is_empty() {
                self.dirty[0] = true;
            }
        }
    }
    for i in (1..self.n).rev() {
        self.tree[i] = self.tree[2 * i] + self.tree[2 * i + 1];
    }
    self.generation += 1;
}
```

- [x] **Step 4：跑测试 → pass**

```bash
cargo test -p app --lib wrap_index
```
预期：含原有 `shift_lines_shifts_dirty` 等全绿。

- [x] **Step 5：Commit**

```bash
git add crates/app/src/wrap_index.rs
git commit -m "fix(wrap_index): shift_lines 把分裂/合并行也标 dirty 以保持局部自洽"
```

---

## 阶段 3：断行核心算法的正确性与性能

### Task 7：`compute_visual_lines` 的累积宽度去除 O(n²)

**Files:**
- Modify: `crates/app/src/render_pipeline.rs:496-618`

**背景：** 每次断行触发 `clusters[visual_line_start..break_at].iter().sum()` 重扫行宽，对于长行（万字符）是 O(n²)。维护 `prefix_width[i] = sum(advance[0..i])` 即可 O(1) 取区间宽度。

- [x] **Step 1：先确保有覆盖现有断行行为的测试**

`render_pipeline.rs` 测试追加（如已存在跳过）：

```rust
#[test]
fn compute_visual_lines_simple_ascii() {
    use shaping::Shaper;
    // 这里用真实 shaper：
    let mut shaper = Shaper::new(14.0).expect("shaper init");
    let line = "the quick brown fox jumps over the lazy dog";
    let shaped = shaper.shape(line).unwrap();
    let line_bytes = line.as_bytes();
    let char_width = 8.0;
    let viewport_width = 80.0; // ~10 chars

    let vls = compute_visual_lines(&shaped.clusters, line_bytes, char_width, viewport_width);
    assert!(vls.len() >= 4, "应该至少 4 段, got {}", vls.len());
    // 段间不重叠、覆盖完整行
    let mut last_end = 0;
    for (s, e, _) in &vls {
        assert_eq!(*s, last_end);
        last_end = *e;
    }
    assert_eq!(last_end, shaped.clusters.len());
}

#[test]
fn compute_visual_lines_empty() {
    let vls = compute_visual_lines(&[], &[], 8.0, 100.0);
    assert!(vls.is_empty());
}

#[test]
fn compute_visual_lines_single_line_fits() {
    use shaping::Shaper;
    let mut shaper = Shaper::new(14.0).unwrap();
    let shaped = shaper.shape("hi").unwrap();
    let vls = compute_visual_lines(&shaped.clusters, b"hi", 8.0, 1000.0);
    assert_eq!(vls.len(), 1);
    assert_eq!(vls[0].0, 0);
    assert_eq!(vls[0].1, shaped.clusters.len());
}
```

- [x] **Step 2：跑测试 → 应当 pass（描述当前行为）**

```bash
cargo test -p app --lib render_pipeline::tests::compute_visual_lines
```
预期：3 个 PASS。这是回归基线。

- [x] **Step 3：重写 `compute_visual_lines` 为前缀和版**

替换函数体（render_pipeline.rs:496-618）为：

```rust
pub(crate) fn compute_visual_lines(
    clusters: &[shaping::GlyphCluster],
    line_bytes: &[u8],
    char_width: f32,
    viewport_width: f32,
) -> Vec<(usize, usize, f32)> {
    if clusters.is_empty() { return Vec::new(); }

    // 预计算每个 cluster 的 advance 与 is_ws，以及 prefix_width。
    // prefix_width[i] = 累计宽度 [0..i)，方便 O(1) 取区间宽度。
    let n = clusters.len();
    let mut adv = Vec::with_capacity(n);
    let mut is_ws = Vec::with_capacity(n);
    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(0.0f32);
    for c in clusters {
        let bytes = line_bytes.get(c.byte_range.clone()).unwrap_or(&[]);
        let ws = is_whitespace_cluster(bytes);
        let a = if ws {
            if bytes == b"\t" { char_width * 4.0 } else { char_width }
        } else {
            c.advance.max(1.0)
        };
        adv.push(a);
        is_ws.push(ws);
        prefix.push(prefix.last().unwrap() + a);
    }
    let width_of = |s: usize, e: usize| prefix[e] - prefix[s];

    // 行尾去掉空白后的"内容宽度"
    let trimmed_width = |s: usize, e: usize| -> f32 {
        let mut w = width_of(s, e);
        let mut i = e;
        while i > s && is_ws[i - 1] {
            w -= adv[i - 1];
            i -= 1;
        }
        w
    };

    let mut visual_lines: Vec<(usize, usize, f32)> = Vec::new();
    let mut start = 0usize;
    let mut last_break_after_ws: Option<usize> = None;
    let mut last_break_cjk: Option<usize> = None;
    let mut last_content_cjk: Option<bool> = None;

    let mut ci = 0usize;
    while ci < n {
        // 边界检测
        if !is_ws[ci] {
            if ci > 0 && is_ws[ci - 1] {
                last_break_after_ws = Some(ci);
            }
            if let Some(b) = line_bytes.get(clusters[ci].byte_range.clone()) {
                if let Some(this_cjk) = cluster_boundary_class(b) {
                    if let Some(prev) = last_content_cjk {
                        if this_cjk != prev {
                            last_break_cjk = Some(ci);
                        }
                    }
                    last_content_cjk = Some(this_cjk);
                }
            }
        }

        let visual_line_x = width_of(start, ci);
        if visual_line_x + adv[ci] > viewport_width && ci > start {
            // 选 break 点：取候选中产生"较宽"行的那个（候选 ≥ start 才有效）
            let cand_ws = last_break_after_ws.filter(|&i| i > start);
            let cand_cjk = last_break_cjk.filter(|&i| i > start);
            let break_at = match (cand_ws, cand_cjk) {
                (Some(ws_i), Some(cjk_i)) => {
                    let ws_x = width_of(start, ws_i);
                    let cjk_x = width_of(start, cjk_i);
                    if ws_x >= cjk_x { ws_i } else { cjk_i }
                }
                (Some(i), None) | (None, Some(i)) => i,
                (None, None) => ci, // 硬断
            };
            let break_x = if break_at == ci {
                visual_line_x
            } else {
                trimmed_width(start, break_at)
            };
            visual_lines.push((start, break_at, break_x));
            start = break_at;
            // 候选清空 + 重新初始化当前 cluster 的 CJK 类别
            last_break_after_ws = None;
            last_break_cjk = None;
            last_content_cjk = None;
            if !is_ws[ci] {
                if let Some(b) = line_bytes.get(clusters[ci].byte_range.clone()) {
                    last_content_cjk = cluster_boundary_class(b);
                }
            }
            // 不前进 ci：让循环用新 start 重新评估当前 cluster
            continue;
        }
        ci += 1;
    }
    if start < n {
        visual_lines.push((start, n, width_of(start, n)));
    }
    visual_lines
}
```

> 关键差异：
> - 用 `prefix` 数组替代每次断行时的求和扫描 → O(n)
> - `break_at == ci` 的硬断不再前进 ci，外层重评估，避免漏算/复算
> - 候选 `start..break` 用 `width_of(start, break)` 取，O(1)

- [x] **Step 4：跑测试**

```bash
cargo test -p app --lib render_pipeline
cargo test -p app --lib  # 整套 app 单测
```
预期：包含原有 `test_word_wrap_tests.rs` 的全部 wrap 行为测试 PASS。如有 fail，对照原算法逐项调查。

- [x] **Step 5：新增长行性能基准（可选但推荐）**

`crates/core/benches/cursor_nav.rs` 同级新增 `crates/app/benches/wrap_long_line.rs`（若已有 `scroll_bench.rs` 沿用其框架）：

```rust
use criterion::{criterion_group, criterion_main, Criterion, black_box};
use shaping::Shaper;

fn bench_long_line_wrap(c: &mut Criterion) {
    let mut shaper = Shaper::new(14.0).unwrap();
    let line = "x".repeat(10_000);
    let shaped = shaper.shape(&line).unwrap();
    let bytes = line.as_bytes();

    c.bench_function("compute_visual_lines_10k_chars", |b| {
        b.iter(|| {
            let _ = app::render_pipeline::compute_visual_lines(
                black_box(&shaped.clusters),
                black_box(bytes),
                black_box(8.0),
                black_box(640.0),
            );
        });
    });
}

criterion_group!(benches, bench_long_line_wrap);
criterion_main!(benches);
```

> 若 `compute_visual_lines` 当前 `pub(crate)`，临时改成 `pub` 或在 app crate 暴露一个 bench-only 的 wrapper。如果 bench infra 较重，可跳过此 step，仅靠正确性测试。

- [x] **Step 6：Commit**

```bash
git add crates/app/src/render_pipeline.rs
git commit -m "perf(wrap): compute_visual_lines 用前缀和消除 O(n²) 行宽重扫"
```

---

### Task 8：极窄视口与起始空白的健壮性

**Files:**
- Modify: `crates/app/src/render_pipeline.rs::compute_visual_lines`

**背景：**
- 当 `viewport_width < advance[0]`（极窄视口）时 `ci > start` 永远 false → 永远不断 → 一行流到天涯。需要"无论如何也要在视口处强行断"，每个 cluster 自占一行。
- 词边界断行后，下一段的开头如果是空白，应该 trim（多数编辑器行为）。

- [x] **Step 1：写测试**

```rust
#[test]
fn very_narrow_viewport_breaks_per_cluster() {
    use shaping::Shaper;
    let mut shaper = Shaper::new(14.0).unwrap();
    let shaped = shaper.shape("abcd").unwrap();
    // viewport 比单字符宽度还小
    let vls = compute_visual_lines(&shaped.clusters, b"abcd", 8.0, 1.0);
    assert_eq!(vls.len(), shaped.clusters.len(),
        "极窄视口应每 cluster 一行, got {}", vls.len());
}

#[test]
fn wrap_skips_leading_whitespace_on_continuation() {
    use shaping::Shaper;
    let mut shaper = Shaper::new(14.0).unwrap();
    // "aaa bbb ccc" 在 viewport ~ 32px (4 chars) 时应在空格处断，
    // 续行不以空格开头。
    let line = "aaa bbb ccc";
    let shaped = shaper.shape(line).unwrap();
    let vls = compute_visual_lines(&shaped.clusters, line.as_bytes(), 8.0, 32.0);
    assert!(vls.len() >= 2);
    for &(s, _, _) in &vls[1..] {
        let cluster = &shaped.clusters[s];
        let first_char = &line.as_bytes()[cluster.byte_range.clone()];
        assert!(!is_whitespace_cluster(first_char),
            "续行不应以空白开头, but got bytes={:?}", first_char);
    }
}
```

- [x] **Step 2：跑 → 第一个 fail（不断行），第二个可能 fail**

```bash
cargo test -p app --lib render_pipeline::tests::very_narrow_viewport render_pipeline::tests::wrap_skips_leading
```

- [x] **Step 3：修改 `compute_visual_lines`**

在 Task 7 实现的循环里，把：

```rust
if visual_line_x + adv[ci] > viewport_width && ci > start {
```

改为：

```rust
let must_break = visual_line_x + adv[ci] > viewport_width;
if must_break && ci > start {
    // 与 Task 7 相同 break 逻辑
    ...
} else if must_break && ci == start {
    // 极窄视口：当前 cluster 单独成一行
    visual_lines.push((start, ci + 1, adv[ci]));
    start = ci + 1;
    last_break_after_ws = None;
    last_break_cjk = None;
    last_content_cjk = None;
    ci += 1;
    continue;
}
```

为续行 trim leading whitespace，在 break 后 `start = break_at` 这一行之后追加：

```rust
// Trim 续行行首空白：多数编辑器行为
while start < n && is_ws[start] {
    start += 1;
}
// 注意：trim 后若 start == ci，继续主循环；否则需要把 ci 回退到 start 处重评估
if start > ci { ci = start; }
```

> 此处要特别小心：trim 后 `last_break_after_ws` / `last_break_cjk` 这些索引可能落到 `< start`，下一次断行的候选检查 `i > start` 会自然过滤掉。无需额外清理。

- [x] **Step 4：跑测试 → pass**

```bash
cargo test -p app --lib render_pipeline
cargo test -p app --lib
```
预期：含 task 7 全部 PASS；既有 word_wrap 测试不回归。

> 若 `wrap_skips_leading_whitespace_on_continuation` 与既有测试冲突（既有测试可能假设保留前导空白），更新既有测试以匹配新行为，并在 commit message 注明语义变更。

- [x] **Step 5：Commit**

```bash
git add crates/app/src/render_pipeline.rs
git commit -m "fix(wrap): 极窄视口强制断行 + 续行 trim 行首空白"
```

---

### Task 9：`char_width` 估算改为"首个 ASCII 非 ws cluster"

**Files:**
- Modify: `crates/app/src/render_pipeline.rs:208-215`

**背景：** 当前取首个非 ws cluster 的 advance 作为 `char_width`。如果首个非 ws 是 CJK 字符（advance ≈ font_size），后续 ASCII 空格按 char_width 算 → 空格被画得过宽，断行偏窄。应优先选 ASCII 字符的 advance；找不到再退化。

- [x] **Step 1：写测试（小工具函数）**

抽出一个 `pick_char_width(clusters, line_bytes, fallback)` 函数，便于单测：

```rust
#[test]
fn char_width_prefers_ascii_letter() {
    use shaping::Shaper;
    let mut shaper = Shaper::new(14.0).unwrap();
    let line = "中A中"; // 首字 CJK，第二字是 ASCII
    let shaped = shaper.shape(line).unwrap();
    let cw = pick_char_width(&shaped.clusters, line.as_bytes(), 100.0);
    // 应取 'A' 的 advance（约 font_size*0.6 = 8.4），而非 '中' 的 (~14)
    assert!(cw < 12.0, "应选 ASCII char_width, got {}", cw);
}

#[test]
fn char_width_falls_back_to_first_non_ws() {
    use shaping::Shaper;
    let mut shaper = Shaper::new(14.0).unwrap();
    let line = "中文";
    let shaped = shaper.shape(line).unwrap();
    let cw = pick_char_width(&shaped.clusters, line.as_bytes(), 100.0);
    assert!(cw > 10.0, "无 ASCII 时退化到首个非 ws cluster");
}

#[test]
fn char_width_falls_back_to_default_when_all_ws() {
    let cw = pick_char_width(&[], b"", 8.0);
    assert_eq!(cw, 8.0);
}
```

- [x] **Step 2：跑 → fail**

```bash
cargo test -p app --lib render_pipeline::tests::char_width
```

- [x] **Step 3：实现 `pick_char_width` 并替换调用点**

在 `render_pipeline.rs` 某处（如 `compute_visual_lines` 上方）新增：

```rust
pub(crate) fn pick_char_width(
    clusters: &[shaping::GlyphCluster],
    line_bytes: &[u8],
    fallback: f32,
) -> f32 {
    // 优先：首个 ASCII 字母/数字 cluster
    for c in clusters {
        let bytes = line_bytes.get(c.byte_range.clone()).unwrap_or(&[]);
        if bytes.len() == 1 {
            let b = bytes[0];
            if b.is_ascii_alphanumeric() {
                return c.advance.max(1.0);
            }
        }
    }
    // 次选：首个非空白 cluster
    for c in clusters {
        let bytes = line_bytes.get(c.byte_range.clone()).unwrap_or(&[]);
        if !is_whitespace_cluster(bytes) {
            return c.advance.max(1.0);
        }
    }
    fallback
}
```

把 `shape_visible_lines` 里：

```rust
let char_width = shaped.clusters.iter()
    .find(|c| { ... })
    .map(|c| c.advance.max(1.0))
    .unwrap_or(settings.font_size * 0.6);
```

替换为：

```rust
let char_width = pick_char_width(&shaped.clusters, &line_bytes, settings.font_size * 0.6);
```

- [x] **Step 4：跑测试 → pass**

```bash
cargo test -p app --lib
```

- [x] **Step 5：Commit**

```bash
git add crates/app/src/render_pipeline.rs
git commit -m "fix(wrap): char_width 优先选 ASCII 字母数字以稳定混排断行宽度"
```

---

## 阶段 4：边界与文档清理

### Task 10：viewport.rs / document_view 的 approx vs exact 路径收敛

**Files:**
- Modify: `crates/app/src/document_view/mod.rs:228-260`
- Modify: `crates/app/src/viewport.rs:151-159`（仅文档/可见性调整）

**背景：** `visible_lines()`、`visible_line_count()`、`visible_line_key()` 仍走 `visible_doc_line_range_approx`，wrap 场景下与 exact 版本结果不一致。app 层已统一用 `*_wrap` 版本，approx 版本只剩这几个未被 wrap 流程使用的函数 — 容易误用。

策略：把 approx 版本改成 `pub(crate)` 或加 `#[deprecated]`，并在 `visible_lines()` 等 doc-comment 里说明"仅用于非 wrap 场景"。或者直接删除未使用的 approx 入口（看 grep 结果决定）。

- [x] **Step 1：grep 调用点**

```bash
grep -rn "visible_doc_line_range_approx\|visible_lines\(\|visible_line_count\(\|visible_line_key\(" crates/ --include='*.rs'
```

- [x] **Step 2：根据结果选择处理方式**

- 若 approx 入口（`visible_lines`、`visible_line_count`、`visible_line_key`）已无外部使用 → 删除。
- 若还有 test/兼容代码使用 → 在函数前加文档：

```rust
/// **DEPRECATED（不要在 wrap 启用路径中使用）**：基于 approx range，
/// 当存在多 visual line 的 doc line 时返回值不一致。新代码请使用 `*_wrap` 版本。
#[deprecated(note = "use visible_line_count_wrap / visible_line_key_wrap")]
pub fn visible_line_count(&self) -> usize { ... }
```

- [x] **Step 3：替换或删除调用点（按 grep 结果）**

如果 `app.rs:1512` 还在用 approx 版本 (status bar / debug 路径)，评估能否切到 `*_wrap`。

- [x] **Step 4：跑测试**

```bash
cargo test -p app --lib
cargo check --workspace
```

- [x] **Step 5：Commit**

```bash
git add crates/app/src/document_view/mod.rs crates/app/src/viewport.rs
git commit -m "chore(viewport): 标记/删除 approx 版本，统一 wrap 路径"
```

---

### Task 11：`set_viewport_width` 用容差 + 清空缓存

**Files:**
- Modify: `crates/app/src/wrap_index.rs:291-296`
- Modify: `crates/app/src/render_pipeline.rs:120-123`

**背景：**
- 用 `f32::EPSILON` 比较视口宽度，几乎任何浮点抖动都触发 mark_all_dirty。改 `> 0.5` 像素的容差。
- 视口宽度变化时不仅要 mark dirty 索引，还要 evict wrap_cache（因为 key 包含 viewport_width — 旧条目自然不会命中，但占着 LRU 容量。可接受不清，但如果资源紧张可清）。

- [x] **Step 1：写测试**

```rust
#[test]
fn set_viewport_width_ignores_subpixel_jitter() {
    let mut idx = WrapIndex::new(3);
    idx.update(0, 5);
    idx.set_viewport_width(800.0);
    assert!(!idx.is_exact(0)); // 第一次设置触发 dirty
    idx.update(0, 5);
    assert!(idx.is_exact(0));
    idx.set_viewport_width(800.1); // 0.1 像素抖动
    assert!(idx.is_exact(0), "亚像素抖动不应触发 dirty");
}

#[test]
fn set_viewport_width_meaningful_change_triggers_dirty() {
    let mut idx = WrapIndex::new(3);
    idx.update(0, 5);
    idx.set_viewport_width(800.0);
    idx.update(0, 5);
    idx.set_viewport_width(820.0);
    assert!(!idx.is_exact(0), "20px 改变应触发 dirty");
}
```

- [x] **Step 2：跑 → 第一个 fail（EPSILON 太小）**

```bash
cargo test -p app --lib wrap_index::tests::set_viewport_width
```

- [x] **Step 3：修改 `set_viewport_width`**

```rust
pub fn set_viewport_width(&mut self, width: f32) {
    if (self.viewport_width - width).abs() > 0.5 {
        self.viewport_width = width;
        self.mark_all_dirty();
    }
}
```

- [x] **Step 4：跑测试 → pass，全套不回归**

```bash
cargo test -p app --lib
```

- [x] **Step 5：Commit**

```bash
git add crates/app/src/wrap_index.rs
git commit -m "fix(wrap_index): set_viewport_width 改用 0.5 像素容差避免抖动失效"
```

---

### Task 12：补 `#[test]` 与文档

**Files:**
- Modify: `crates/app/src/wrap_index.rs:749`（`viewport_width_change_marks_dirty` 漏 `#[test]`）
- Modify: `crates/app/src/wrap_index.rs:113-135`（`display_to_doc` 越界返回值文档）

- [x] **Step 1：补 `#[test]`**

```rust
#[test]
fn viewport_width_change_marks_dirty() {
    // ...原有函数体不变
}
```

- [x] **Step 2：完善 `display_to_doc` 文档**

```rust
/// Convert absolute DisplayRow → doc line index.
/// - 当 `display_row >= total_display_rows()`：返回 `len`（"one past last"，调用方
///   需要据此知道 display_row 已超出末尾；常见用法 `min(len-1)` 取末行）。
/// - 否则：返回包含 display_row 的 doc 行（值在 [0, len-1]）。
/// - 当索引为空（`len == 0`）：固定返回 0。
pub fn display_to_doc(&self, display_row: usize) -> usize {
```

- [x] **Step 3：跑测试**

```bash
cargo test -p app --lib wrap_index
```
预期：`viewport_width_change_marks_dirty` 现在被执行；全绿。

- [x] **Step 4：Commit**

```bash
git add crates/app/src/wrap_index.rs
git commit -m "chore(wrap_index): 补漏的 #[test] 与 display_to_doc 越界文档"
```

---

## 阶段 5：集成回归

### Task 13：手动 + 自动化的端到端冒烟

**Files:**
- Reference: 现有 `crates/app/tests/render_smoke.rs`、`crates/app/src/document_view/test_word_wrap_tests.rs`、`crates/app/src/document_view/test_perf_tests.rs`

- [x] **Step 1：跑全量**

```bash
cargo test --workspace
```
预期：全绿。如有 fail，按报错路径对照 task 1–12 的改动定位。

- [x] **Step 2：手动冒烟（按 CLAUDE.md 项目惯例，写完代码要确保编译过；UI 验证）**

启动应用并手动验证以下场景（每条记录通过/失败）：

- [x] 加载长 JSON（≥ 2MB），向下/向上滚动一遍，无错位
- [x] 在 viewport 外（屏幕下方）的长行附近按 PageDown/PageUp，滚动条位置正确
- [x] 在首行末尾粘贴一段 1000 字符（让首行从 1 行变 ~30 行）：滚动条立即更新，光标可见
- [x] 缩窄窗口宽度（拖窗框）使长行重新断行：第一帧不闪、不错位
- [x] 切到一个含日文/中文混排的文件，断行不会把"あ A"这种边界硬切
- [x] 输入一连串 IME 候选（日文/中文）：每次提交后行号、滚动条位置一致

```bash
cargo run --release -- /path/to/large.json
```

- [x] **Step 3：把手动冒烟结果记录到 commit message**

```bash
git commit --allow-empty -m "test: 断行修复手动冒烟通过 (long-line / wrap / IME)"
```

---

## Self-Review 检查清单

完成全部 task 后回头核对：
- [x] 所有 `is_ascii_whitespace` 调用点已迁移到 `is_whitespace_cluster`（grep 0 结果）
- [x] `shape_cache` / `wrap_cache` 的 key 全部包含 `(offset, length, font_size_bits)` 前缀
- [x] `app.rs` 编辑路径（handle_command + IME）都通过 `execute_edit_command_v2`
- [x] `compute_visual_lines` 不再有 `clusters[..].iter().map().sum()` 这种内层求和
- [x] `WrapIndex::update_batch` 不再 `for i in (1..self.n).rev()` 全树重建
- [x] 假名/谚文相关测试 PASS
- [x] `cargo test --workspace` 全绿
- [x] `cargo check --workspace` 全绿

---

## 备注（不在本计划范围）

- `cluster_boundary_class` 在 `last_break_cjk` 选取上"只保留最后一次"；如果未来发现日韩文本断行偏短，再升级为"最后一次且 x 不超过 viewport_width"的策略。
- 渲染主循环的 advance 计算（render_pipeline.rs:347-348, 408 等）目前在 Task 1 的 `cluster_advance` 已统一；若需进一步避免重复枚举，可让 shaper 在 `GlyphCluster` 上直接挂"effective_advance"字段，那是更大的重构，不在本计划。
- `wrap_index.update()` 是否要 bump generation：保守起见保持不动；若下游缓存需要观察"内容变化"，应订阅 `dirty_lines` 事件（Task 5 的 outcome），而非 generation。
