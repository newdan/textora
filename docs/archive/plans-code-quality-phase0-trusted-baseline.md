# Trusted Workspace Baseline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 接通搜索替换后端、修复过时 benchmark 和测试标记，使 workspace 测试及 all-targets 编译恢复可信基线。

**Architecture:** 恢复仓库历史中已验证过的 ICU UTF-16 regex 适配层，但让 `TextBuffer` 显式提供字节快照，避免 production stub 和伪泛型接口；benchmark 仅迁移到现有 ui/DocumentView 稳定入口，不改变测量内容。本计划不重构 app/UI 大模块。

**Tech Stack:** Rust、动态加载 ICU、Cargo tests、Criterion。

---

### Task 1: 固化搜索替换失败与边界行为

**Files:**
- Modify: `crates/core/src/buffer/text_buffer_tests.rs:527-626`

- [x] **Step 1: 保留现有 8 个回归测试，并增加 Unicode 与非法正则用例**

在 `replace_all_and_undo_then_redo` 后加入：

```rust
#[test]
fn regex_replace_preserves_utf8_byte_ranges() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.write_raw("a中🙂b 中🙂".as_bytes());

    buf.find_and_replace_all(
        r"(中)(🙂)",
        SearchOptions { use_regex: true, ..Default::default() },
        b"$2$1",
    )
    .unwrap();

    assert_eq!(buffer_contents(&mut buf), "a🙂中b 🙂中");
}

#[test]
fn invalid_regex_returns_error_without_mutating_buffer() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.write_raw(b"unchanged");

    let result = buf.find_and_replace_all(
        "(",
        SearchOptions { use_regex: true, ..Default::default() },
        b"x",
    );

    assert!(result.is_err());
    assert_eq!(buffer_contents(&mut buf), "unchanged");
}
```

- [x] **Step 2: 运行测试并确认 stub 复现**

Run:

```bash
cargo test -p edit-plus-core --lib buffer::text_buffer::tests::regex_replace_preserves_utf8_byte_ranges -- --exact
cargo test -p edit-plus-core --lib buffer::text_buffer::tests::undo_after_replace_all_single_step -- --exact
```

Expected: 两条测试均在 `.unwrap()` 处失败，错误为 `Error(0)`；不是编译错误。

- [x] **Step 3: 提交仅测试的红色基线**

```bash
git add crates/core/src/buffer/text_buffer_tests.rs
git commit -m "test(core): cover replace backend byte ranges and errors"
```

### Task 2: 恢复真实 ICU Regex/Text 后端

**Files:**
- Modify: `crates/core/src/icu.rs:547-608,925-996`
- Modify: `crates/core/src/buffer/search.rs:184-266`

- [x] **Step 1: 读取最后一版真实实现作为确定来源**

Run:

```bash
git show 2d07df5c:crates/core/src/icu.rs | sed -n '930,1260p'
git show 2d07df5c:crates/core/src/buffer/search.rs | sed -n '215,275p'
```

Expected: 输出包含 `build_utf16_mapping`、持有 `utf8/utf16/utf16_to_byte` 的 `Text`、持有 ICU handle 的 `Regex`、`uregex_setText` 绑定与 `Drop`。

- [x] **Step 2: 用历史真实实现完整替换 stub，并恢复 `uregex_setText` 动态符号**

实现必须与来源保持下列接口；不得保留 `pub struct Regex;` / `pub struct Text;`：

```rust
pub struct Text {
    pub(crate) utf8: Vec<u8>,
    pub(crate) utf16: Vec<u16>,
    pub(crate) utf16_to_byte: Vec<usize>,
}

impl Text {
    pub unsafe fn new(bytes: &[u8]) -> Result<Self>;
    pub fn rebuild(&mut self, bytes: &[u8]) -> Result<()>;
}

pub struct Regex {
    re: *mut icu_ffi::URegularExpression,
    utf16_to_byte: Vec<usize>,
    byte_len: usize,
}

impl Regex {
    pub unsafe fn new(pattern: &str, flags: u32, text: &Text) -> Result<Self>;
    pub unsafe fn set_text(&mut self, text: &Text, offset: usize) -> Result<()>;
    pub fn reset(&mut self, offset: usize) -> Result<()>;
    pub fn find_next(&mut self) -> Result<Option<std::ops::Range<usize>>>;
    pub fn group_count(&self) -> Result<i32>;
    pub fn group(&self, index: i32) -> Result<Option<std::ops::Range<usize>>>;
}
```

相较历史代码必须做两处收紧：`Text::rebuild` 不吞掉 UTF-8 错误；所有 ICU status 失败均返回 `Err(status.as_error())`，不能折叠为 `None`。`Regex::new` 设置 500ms time limit；`Drop` 调 `uregex_close`。

- [x] **Step 3: 让 TextBuffer 显式提取完整字节快照**

在 `find_construct_search` 中使用：

```rust
let doc_len = self.buffer.len();
let mut doc_bytes = Vec::with_capacity(doc_len);
self.buffer.extract_raw(0..doc_len, &mut doc_bytes, 0);
let text = unsafe { icu::Text::new(&doc_bytes)? };
let regex = unsafe { icu::Regex::new(&sanitized_pattern, flags, &text)? };
```

在 generation 改变分支使用：

```rust
let doc_len = self.buffer.len();
let mut doc_bytes = Vec::with_capacity(doc_len);
self.buffer.extract_raw(0..doc_len, &mut doc_bytes, 0);
search.text.rebuild(&doc_bytes).ok()?;
unsafe { search.regex.set_text(&search.text, offset).ok()? };
```

同步把 `find_select_next` 改为：

```rust
fn find_select_next(
    &mut self,
    search: &mut ActiveSearch,
    offset: usize,
    wrap: bool,
) -> icu::Result<Option<Range<usize>>>;
```

函数内部对 `set_text/reset/find_next` 使用 `?`；public `find_and_select/find_and_replace/find_and_replace_all` 对调用使用 `?`，replace-all 循环写成 `while let Some(range) = self.find_select_next(&mut search, offset, false)?`。`find_parse_replacement` 和 `find_fill_replacement` 同样返回 `icu::Result<_>`，对 `group_count/group` 使用 `?`。只有成功得到 `Ok(None)` 才设置 `no_matches = true`，ICU 失败必须原样返回。

- [x] **Step 4: 运行全部替换与 ICU 定向测试**

Run:

```bash
cargo test -p edit-plus-core --lib buffer::text_buffer::tests:: -- --nocapture
cargo test -p edit-plus-core --lib icu::tests:: -- --nocapture
```

Expected: 现有 8 类替换回归、Unicode 新用例、非法正则用例全部 PASS；无 `Error(0)`。

- [x] **Step 5: 确认 production stub 已消失并提交**

Run:

```bash
rg -n "stub|Err\(Error\(0\)\)" crates/core/src/icu.rs crates/core/src/buffer/search.rs
```

Expected: 无输出。

```bash
git add crates/core/src/icu.rs crates/core/src/buffer/search.rs
git commit -m "fix(core): restore functional ICU search backend"
```

### Task 3: 迁移 tab 与 scroll benchmark

**Files:**
- Modify: `crates/app/benches/tab_bench.rs:17-46`
- Modify: `crates/app/benches/scroll_bench.rs:14-70,187-193`
- Modify: `crates/app/src/document_view/mod.rs:290-300`

- [x] **Step 1: 将 tab benchmark 指向 ui crate**

顶部加入：

```rust
use ui::widgets::tab_bar::{TabBarCtx, TabInfo, layout_tabs, tab_bar_height};
```

把所有 `edit_plus_app::tab_bar::` 前缀移除，并保持 `TabInfo`、`TabBarCtx` 字段与 `crates/ui/src/widgets/tab_bar/types.rs` 一致。

- [x] **Step 2: 先验证 tab bench 恢复编译**

Run: `cargo check -p edit-plus-app --bench tab_bench`

Expected: PASS；不得有 `cannot find tab_bar` 或 `unused_mut`。

- [x] **Step 3: 将 scroll benchmark 迁移到 display viewport 和 TextBuffer cursor**

定义 bench 内部 helper：

```rust
fn scroll_one_doc_line(dv: &mut edit_plus_app::document_view::DocumentView) {
    let map = edit_plus_app::display_line_map::DisplayLineMap::new(&dv.display.display_map);
    dv.display.viewport.scroll_doc_lines(1, &map);
}
```

在 `DocumentView` 增加保持 TextBuffer 封装的只读 accessor：

```rust
pub fn cursor_offset(&self) -> usize {
    self.tb.cursor_offset()
}
```

用 `scroll_one_doc_line(&mut dv)` 替换 `dv.scroll_down(1)`；用 `dv.display.viewport` 替换 `dv.viewport`；用 `dv.cursor_offset()` 替换 `dv.cursor_offset`。

- [x] **Step 4: 验证两个 benchmark 和 all-targets**

Run:

```bash
cargo check -p edit-plus-app --benches
cargo check --workspace --all-targets
```

Expected: 两条命令退出码 0；原 13 个错误全部消失。

- [x] **Step 5: 提交 benchmark 迁移**

```bash
git add crates/app/benches/tab_bench.rs crates/app/benches/scroll_bench.rs crates/app/src/document_view/mod.rs
git commit -m "fix(bench): migrate tab and scroll benchmarks"
```

### Task 4: 修正测试注册和性能测试分层

**Files:**
- Modify: `crates/app/src/commands.rs:1098-1150`
- Modify: `crates/app/src/snap_tree.rs:390-430`
- Create: `crates/app/benches/snap_tree_bench.rs`

- [x] **Step 1: 修复重复与遗漏的 test attribute**

删除 `toggle_comment` 上重复的一个 `#[test]`，并在 `indent_home_in_visual_line` 前添加一个 `#[test]`。

- [x] **Step 2: 先验证测试清单恰好注册一次**

Run:

```bash
cargo test -p edit-plus-app --lib -- --list | rg "toggle_comment|indent_home_in_visual_line"
```

Expected: 两个测试名各出现一次。

- [x] **Step 3: 将墙钟性能测试移到 Criterion bench**

删除普通测试 `bench_splice_18000_entries`，在新文件中用：

```rust
use criterion::{Criterion, criterion_group, criterion_main};
use edit_plus_app::snap_tree::{DisplayLineEntry, SnapTree};

fn splice_18000_entries(c: &mut Criterion) {
    let entries: Vec<_> = (0..18_000)
        .map(|i| DisplayLineEntry::placeholder(i * 200, 200, i as u64, 1))
        .collect();
    c.bench_function("snap_tree/splice_18000_entries", |b| {
        b.iter_batched(
            || SnapTree::from_entries(entries.clone()),
            |mut tree| {
                let replacement = DisplayLineEntry::placeholder(1_000_000, 200, 9_999, 1);
                criterion::black_box(tree.splice(5_000..5_001, vec![replacement]));
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, splice_18000_entries);
criterion_main!(benches);
```

在 `crates/app/Cargo.toml` 注册 bench 会形成第 4 个文件，因此作为紧随其后的独立提交执行：

```toml
[[bench]]
name = "snap_tree_bench"
harness = false
```

- [x] **Step 4: 验证测试不再写固定 /tmp 文件，bench 可编译**

Run:

```bash
rg -n "/tmp/snap_bench.txt|bench_splice_18000_entries" crates/app/src
cargo check -p edit-plus-app --bench snap_tree_bench
cargo test -p edit-plus-app --lib commands::tests::indent_home_in_visual_line -- --exact
```

Expected: `rg` 无输出；后两条 PASS。

- [x] **Step 5: 分两次提交，保持每次不超过 3 个文件**

```bash
git add crates/app/src/commands.rs crates/app/src/snap_tree.rs crates/app/benches/snap_tree_bench.rs
git commit -m "test(app): separate correctness tests from snap tree benchmark"
git add crates/app/Cargo.toml
git commit -m "bench(app): register snap tree benchmark"
```

### Task 5: Phase 0 总验收

**Files:**
- No files changed.

- [x] **Step 1: 运行可信基线**

```bash
cargo test --workspace
cargo check --workspace --all-targets
```

Expected: 两条命令退出码均为 0。warning 留给 Phase 1，但不得包含重复 attribute、未知 benchmark API 或失败测试。
