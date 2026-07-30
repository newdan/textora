# Unified Edit Transaction Rewrite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用一套通用、类型安全、可原子执行的编辑事务链路替换基础编辑器与 Markdown WYSIWYG 当前分裂的普通命令、augmentation 和递归分派路径，并系统修复回车、逐行删除、结构化删除、选区替换、空行导航及尾随空行布局问题。

**Architecture:** `ui` 只定义纯数据 `EditIntent / EditRequest / EditPlan / EditTransaction` 协议；`app` 从 `DocumentView` 构造请求、生成默认文本事务、验证并原子执行事务；Markdown plugin 只根据自身缓存的源码、解析结果和 `SourceLineMap` 返回结构化计划。`DocumentView` 继续是源码、光标、选区、撤销历史的唯一真相源，plugin 中的光标与选区只作为渲染镜像。

**Tech Stack:** Rust、现有 `core::buffer::TextBuffer`、`DocumentView`、`ui::plugin::ViewPlugin`、`pulldown-cmark`、Markdown `SourceLineMap`、Cargo workspace tests。

## Global Constraints

- 产品名保持 `textora`，Markdown crate 包名保持 `textora-markdown`。
- `crates/ui` 只能定义纯数据协议和 UI 抽象，不能依赖 `DocumentView`、Workspace、App Commands 或 App Events。
- Markdown 语义只能存在于 `crates/markdown`；`crates/app` 只做请求构造、默认事务规划、事务执行和状态同步。
- `DocumentView` 是源码、cursor、selection、undo/redo 的唯一权威；plugin 不能直接修改文档。
- 每个实施任务最多修改 3 个文件；超过 3 个文件必须拆成独立任务并单独验证。
- 所有行为变更严格执行 TDD：先写失败测试并确认失败原因，再写最小实现，再运行局部与回归测试。
- 同一个缺陷连续两次修复仍失败时停止叠加补丁，重新检查事务协议或源码—视觉映射架构。
- 不引入新的第三方依赖；复用 `TextBuffer` 现有 grapheme 导航和 pulldown-cmark 解析结果。
- 禁止新增宽泛命名，如 `data`、`info`、`temp`、`res`、`flag`；事务类型和上下文类型必须精确自解释。
- Rust 中互斥状态必须用 enum 表达，不允许使用多个 bool 或多个 Option 的组合编码事务状态。
- 禁止新增无理由 `.unwrap()`；测试夹具使用 `.expect("具体不变量")`。
- 每阶段完成后运行 `cargo fmt --all -- --check` 和该阶段涉及 crate 的编译、测试。
- 最终运行 `./scripts/verify.sh`。

---

## 1. 行为规范

### 1.1 通用事务不变量

1. 每次文本编辑最多产生一个连续源码替换区间；跨结构选择由 policy 生成覆盖该连续区间的新文本。
2. 普通插入或删除事务完成后必须清空 selection。
3. `cursor_after` 必须位于替换后的源码长度内、UTF-8 char boundary 和 grapheme boundary。
4. 删除事务默认满足 `cursor_after == replacement.range.start`；只有 `EditPlan::MoveCursor` 允许无文本修改地移动光标。
5. `EditPlan::Consume` 只能用于明确禁止源码变化的结构操作，例如表格最后一行 Enter。
6. `UseDefault` 只能由 plugin policy 返回；App 收到后必须转换成默认 `EditTransaction`，不能重新进入旧 `EditCommand` 文本修改分支。
7. 一次事务只产生一份 `EditOutcome`、一次 undo history entry、一次 source generation 增量和一次 plugin full sync。

### 1.2 标准富文本行为

| 上下文 | Enter | Backspace | DeleteForward | Tab / Shift+Tab |
|---|---|---|---|---|
| 普通段落中间 | 在光标处拆成两个段落，源码插入 `\n\n` | 删除前一 grapheme | 删除后一 grapheme | 默认缩进 |
| 普通段落边界 | 创建或合并相邻段落 | 每次只删除一个相邻逻辑换行 | 每次只删除一个相邻逻辑换行 | 默认缩进 |
| 标题中间 | 光标前保留标题，光标后成为普通段落 | 删除前一 grapheme | 删除后一 grapheme | 默认缩进 |
| 标题 marker 后 | Backspace 将标题降为普通段落 | 删除整个当前标题 marker，但不删除正文 | DeleteForward 删除正文首 grapheme | 默认缩进 |
| 非空列表项 | 插入同层新 item；有序列表使用当前序号 + 1 | marker 后逐级降低一层 | 正文内普通删除 | Tab 缩进一层，Shift+Tab 降低一层 |
| 空列表项 | 退出当前列表层级 | 逐级退出当前层级 | 删除后续内容 | Tab/Shift+Tab 调整层级 |
| 非空引用行 | 插入同层 `> ` 行 | marker 后只退出一层引用 | 正文内普通删除 | Tab 增加一层，Shift+Tab 减少一层 |
| 空引用行 | 退出当前引用层级 | 只退出一层 | 删除后续内容 | 调整引用层级 |
| 代码块 | 插入单个逻辑换行并保留代码块 | 默认删除 | 默认删除 | 保持现有缩进规则 |
| 表格单元格 | 移动到下一行同列；最后一行 Consume | 不得删除 pipe/分隔行结构 | 不得删除 pipe/分隔行结构 | 移动到相邻单元格 |
| 可编辑空行 | 增加一条空行 | 只删除前一个逻辑换行 | 只删除后一个逻辑换行 | 默认缩进 |
| 隐藏块分隔符 | 光标不能停留 | 不直接处理 | 不直接处理 | 不处理 |

### 1.3 选区规则

- 选区完全位于单一普通段落、标题正文、列表正文、引用正文、代码块或表格单元格正文时：先在内存中构造“删除选区后的虚拟源码”，再按 selection start 规划 intent，最终合并为一个连续替换事务。
- 选区跨越 Markdown marker 时：policy 必须保护结构 marker；不能把表格 pipe、fence、列表父级 marker 作为普通正文删除。
- InsertText/Paste 替换选区时保持外围块结构。
- Enter 替换选区时按 selection start 所在块创建段落或结构行。
- Backspace/DeleteForward 在非空选区上行为一致：删除受保护规范化后的正文范围，不再执行额外向前或向后删除。

---

## 2. 目标数据流

```text
Keyboard / IME / Paste / Selection Edit
  -> EditCommand 仅做输入映射
  -> App::build_edit_request
  -> active_plugin.edit_policy().plan_edit(request)
       -> UseDefault: app::default_edit_plan(request, doc)
       -> Apply(transaction)
       -> MoveCursor(cursor_update)
       -> Consume
  -> app::execute_edit_plan
       -> validate ranges and cursor
       -> DocumentView atomic replacement
       -> EditOutcome
  -> App cache invalidation
  -> App::sync_plugin_state exactly once
  -> plugin source/cursor/selection mirror
```

### 2.1 目标协议

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditIntent {
    InsertText(String),
    InsertParagraphBreak,
    DeleteBackward,
    DeleteForward,
    Indent,
    Outdent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditRequest {
    pub source_generation: u32,
    pub cursor_byte: usize,
    pub selection: Option<std::ops::Range<usize>>,
    pub intent: EditIntent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextReplacement {
    pub range: std::ops::Range<usize>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditTransaction {
    pub replacement: TextReplacement,
    pub cursor_after: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorUpdate {
    pub cursor_after: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditPlan {
    UseDefault,
    Apply(EditTransaction),
    MoveCursor(CursorUpdate),
    Consume,
}

pub trait EditPolicy {
    fn plan_edit(&self, request: &EditRequest) -> EditPlan;
}
```

`ViewPlugin` 新增：

```rust
fn edit_policy(&self) -> &dyn EditPolicy {
    &NoopEditPolicy
}
```

在兼容阶段保留 `augmenter()`；Task 14 完成后删除 `AugmentKind`、`EditAugmentation`、`AugmentContext`、`EditAugmenter` 和 `NoopAugmenter`。

---

## 3. 文件职责映射

| 文件 | 最终职责 |
|---|---|
| `crates/ui/src/plugin.rs` | 通用编辑意图、请求、计划、事务和 `EditPolicy` 纯数据协议 |
| `crates/core/src/buffer/text_buffer.rs` | 不改变真实 cursor 的 grapheme 边界只读查询 |
| `crates/app/src/edit_transaction.rs` | EditCommand→EditIntent、默认计划、事务验证和原子执行 |
| `crates/app/src/dispatch/editor.rs` | 所有文本修改命令进入统一事务入口 |
| `crates/app/src/commands.rs` | 保留导航、搜索、撤销重做等非事务命令；最终移除文本修改的重复实现 |
| `crates/markdown/src/edit_context.rs` | Markdown block/line/marker/selection 结构上下文 |
| `crates/markdown/src/edit_policy.rs` | Markdown `EditPolicy`，只生成事务计划 |
| `crates/markdown/src/layout/source_line_map.rs` | 每条源码行的角色、run 位置和布局几何唯一映射 |
| `crates/markdown/src/layout/types.rs` | 使用 SourceLineMap 的几何输出，不再独立计算空行高度 |
| `crates/markdown/src/view.rs` | 渲染、hit-test、视觉导航和 plugin 接线，不再包含编辑语义分类器 |
| `crates/app/src/dispatch/wysiwyg.rs` | 最终只保留视觉导航 helper，删除 augmented edit 递归分派 |

### 3.1 测试夹具约定

各任务中的测试 helper 必须放在对应文件的 `#[cfg(test)]` module 内，不能进入生产 API。统一使用以下签名，避免任务之间出现同义不同名：

```rust
fn document_from_text(text: &str) -> DocumentView {
    DocumentView::new(text.split('\n').map(str::to_owned).collect(), 80, 10.0)
}

fn request_at(cursor_byte: usize, intent: EditIntent) -> EditRequest {
    EditRequest { source_generation: 1, cursor_byte, selection: None, intent }
}

fn request_with_selection(selection: Range<usize>, intent: EditIntent) -> EditRequest {
    EditRequest {
        source_generation: 1,
        cursor_byte: selection.end,
        selection: Some(selection),
        intent,
    }
}

fn apply(range: Range<usize>, text: &str, cursor_after: usize) -> EditPlan {
    EditPlan::Apply(EditTransaction {
        replacement: TextReplacement { range, text: text.to_owned() },
        cursor_after,
    })
}

fn policy_for(source: &str) -> MarkdownEditPolicy {
    MarkdownEditPolicy::new(source.to_owned(), 1)
}

fn apply_plan_to_text(source: &str, plan: EditPlan) -> String {
    let EditPlan::Apply(transaction) = plan else {
        return source.to_owned();
    };
    let mut updated_source = source.to_owned();
    updated_source.replace_range(transaction.replacement.range, &transaction.replacement.text);
    updated_source
}
```

core buffer tests 复用 `crates/core/src/buffer/text_buffer_tests.rs` 已有构造方式，并新增唯一 helper：

```rust
fn text_buffer_from_text(text: &str) -> TextBuffer {
    let mut buffer = TextBuffer::new(false).expect("test buffer must be created");
    buffer.write_raw(text.as_bytes());
    buffer
}
```

App policy 路由测试扩展 `crates/app/src/app_tests.rs` 已有 `RecordingWysiwygPlugin`，把旧 `augmentation` 字段替换为 `edit_plan: EditPlan`，并新增 `recorded_edit_requests: Vec<EditRequest>`；不再创建第二套 recording plugin。

---

## 4. 分阶段执行

### Task 0: 隔离工作区与基线

**Files:**
- No file changes.

**Interfaces:**
- Consumes: 当前 HEAD 和用户未提交修改。
- Produces: 隔离分支 `codex/unified-edit-transaction-rewrite`，基线测试记录。

- [ ] **Step 1: 创建隔离 worktree**

先按 `using-git-worktrees` 技能检查当前是否已经位于 linked worktree。若不是，确认 `.worktrees` 被 gitignore 后创建：

```bash
git check-ignore -q .worktrees
git worktree add .worktrees/unified-edit-transaction-rewrite -b codex/unified-edit-transaction-rewrite
```

Expected: 新 worktree 干净，当前工作区的 `layout/shaping.rs`、`issues.md`、`test_data/sample.mmap.md` 未提交修改不进入新 worktree。

- [ ] **Step 2: 运行基线编译和测试**

```bash
cargo check -p textora-ui -p textora-markdown -p textora-app
cargo test -p textora-ui
cargo test -p textora-markdown
cargo test -p textora-app
```

Expected: 全部 exit 0；若基线失败，停止实施并记录失败测试，不进入 Task 1。

---

### Task 1: 在 ui 定义类型安全编辑事务协议

**Files:**
- Modify: `crates/ui/src/plugin.rs:65-132`
- Test: `crates/ui/src/plugin.rs` tests module

**Interfaces:**
- Consumes: `std::ops::Range`、现有 `ViewPlugin`。
- Produces: `EditIntent`、`EditRequest`、`TextReplacement`、`EditTransaction`、`CursorUpdate`、`EditPlan`、`EditPolicy`、`NoopEditPolicy`、`ViewPlugin::edit_policy()`。

- [ ] **Step 1: 写失败的协议测试**

在 `plugin.rs` tests module 添加：

```rust
#[test]
fn noop_edit_policy_requests_default_transaction() {
    let request = EditRequest {
        source_generation: 7,
        cursor_byte: 4,
        selection: Some(1..4),
        intent: EditIntent::DeleteBackward,
    };

    assert_eq!(NoopEditPolicy.plan_edit(&request), EditPlan::UseDefault);
}

#[test]
fn transaction_state_is_expressed_by_enum_variants() {
    let transaction = EditTransaction {
        replacement: TextReplacement { range: 4..4, text: "\n\n".into() },
        cursor_after: 6,
    };

    assert!(matches!(EditPlan::Apply(transaction), EditPlan::Apply(_)));
    assert!(matches!(
        EditPlan::MoveCursor(CursorUpdate { cursor_after: 9 }),
        EditPlan::MoveCursor(_)
    ));
}
```

- [ ] **Step 2: 确认测试按预期失败**

Run: `cargo test -p textora-ui noop_edit_policy_requests_default_transaction transaction_state_is_expressed_by_enum_variants`

Expected: compile failure，缺少 `EditRequest` 或 `EditPlan`。

- [ ] **Step 3: 添加目标协议和默认 policy**

按本文 §2.1 的完整类型定义添加协议，并实现：

```rust
pub struct NoopEditPolicy;

impl EditPolicy for NoopEditPolicy {
    fn plan_edit(&self, _request: &EditRequest) -> EditPlan {
        EditPlan::UseDefault
    }
}
```

在 `ViewPlugin` 中添加本文 §2.1 的 `edit_policy()` 默认方法。此任务不删除 augmentation 类型。

- [ ] **Step 4: 验证 ui**

Run: `cargo fmt --all -- --check && cargo test -p textora-ui && cargo check -p textora-ui`

Expected: exit 0。

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/plugin.rs
git commit -m "feat(ui): define typed edit transaction protocol"
```

---

### Task 2: 为 TextBuffer 增加只读 grapheme 边界查询

**Files:**
- Modify: `crates/core/src/buffer/text_buffer.rs:560-590`
- Test: `crates/core/src/buffer/text_buffer_tests.rs`

**Interfaces:**
- Consumes: `CursorMovement::Grapheme`、`cursor_move_to_byte_internal`、`cursor_move_delta_internal`。
- Produces: `TextBuffer::grapheme_boundary_delta(&self, offset: ByteIndex, delta: isize) -> ByteIndex`、`TextBuffer::is_grapheme_boundary(&self, offset: ByteIndex) -> bool`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn grapheme_boundary_delta_does_not_mutate_real_cursor() {
    let emoji = "👨\u{200D}👩\u{200D}👧";
    let mut buffer = text_buffer_from_text(&format!("a{emoji}b"));
    let emoji_end = 1 + emoji.len();
    buffer.cursor_move_to_byte(ByteIndex(emoji_end));

    let target = buffer.grapheme_boundary_delta(ByteIndex(emoji_end), -1);

    assert_eq!(target, ByteIndex(1));
    assert_eq!(buffer.cursor().offset, ByteIndex(emoji_end));
}

#[test]
fn grapheme_boundary_query_rejects_middle_of_cluster() {
    let emoji = "👨\u{200D}👩\u{200D}👧";
    let buffer = text_buffer_from_text(emoji);

    assert!(buffer.is_grapheme_boundary(ByteIndex(0)));
    assert!(buffer.is_grapheme_boundary(ByteIndex(emoji.len())));
    assert!(!buffer.is_grapheme_boundary(ByteIndex(4)));
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p textora-core grapheme_boundary_delta_does_not_mutate_real_cursor grapheme_boundary_query_rejects_middle_of_cluster`

Expected: compile failure，方法不存在。

- [ ] **Step 3: 实现只读查询**

```rust
pub fn grapheme_boundary_delta(&self, offset: ByteIndex, delta: isize) -> ByteIndex {
    let cursor = self.cursor_move_to_byte_internal(self.cursor, offset);
    self.cursor_move_delta_internal(cursor, CursorMovement::Grapheme, delta).offset
}

pub fn is_grapheme_boundary(&self, offset: ByteIndex) -> bool {
    self.cursor_move_to_byte_internal(self.cursor, offset).offset == offset
}
```

该函数不能调用 `set_cursor`，不能改变 selection 或 generation。

- [ ] **Step 4: 验证 core**

Run: `cargo fmt --all -- --check && cargo test -p textora-core grapheme_boundary`

Expected: 新测试通过；ZWJ、组合字符、CRLF 边界测试无回归。

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/buffer/text_buffer.rs crates/core/src/buffer/text_buffer_tests.rs
git commit -m "feat(core): expose read-only grapheme boundary query"
```

---

### Task 3: 建立 App 默认事务规划与验证器

**Files:**
- Create: `crates/app/src/edit_transaction.rs`
- Modify: `crates/app/src/lib.rs:78-85`
- Test: `crates/app/src/edit_transaction.rs` tests module

**Interfaces:**
- Consumes: Task 1 协议、Task 2 grapheme 查询、`DocumentView`。
- Produces: `edit_intent_for_command`、`build_edit_request`、`default_edit_plan`、`validate_edit_transaction`。

- [ ] **Step 1: 写默认规划失败测试**

```rust
#[test]
fn default_backspace_deletes_one_grapheme_and_keeps_cursor_at_range_start() {
    let emoji = "👨\u{200D}👩\u{200D}👧";
    let mut doc = document_from_text(&format!("a{emoji}b"));
    let emoji_end = 1 + emoji.len();
    doc.cursor_move_to_offset(emoji_end);
    let request = build_edit_request(&doc, EditIntent::DeleteBackward);

    let plan = default_edit_plan(&request, &doc);

    assert_eq!(
        plan,
        EditPlan::Apply(EditTransaction {
            replacement: TextReplacement { range: 1..emoji_end, text: String::new() },
            cursor_after: 1,
        })
    );
}

#[test]
fn default_delete_with_selection_only_deletes_selection() {
    let mut doc = document_from_text("abcdef");
    doc.cursor_move_to_offset(5);
    doc.cursor_mut().selection_anchor = Some(2);
    let request = build_edit_request(&doc, EditIntent::DeleteForward);

    assert_eq!(
        default_edit_plan(&request, &doc),
        EditPlan::Apply(EditTransaction {
            replacement: TextReplacement { range: 2..5, text: String::new() },
            cursor_after: 2,
        })
    );
}

#[test]
fn validator_rejects_cursor_after_final_text() {
    let transaction = EditTransaction {
        replacement: TextReplacement { range: 1..2, text: String::new() },
        cursor_after: 9,
    };

    assert_eq!(
        validate_edit_transaction("abc", &transaction),
        Err(EditTransactionError::CursorOutOfBounds { cursor_after: 9, final_len: 2 })
    );
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p textora-app edit_transaction::tests`

Expected: compile failure，module 和函数不存在。

- [ ] **Step 3: 实现请求构造和默认规划**

默认规划必须完整覆盖：

```rust
match &request.intent {
    EditIntent::InsertText(text) => replace_selection_or_cursor(request, text.clone()),
    EditIntent::InsertParagraphBreak => replace_selection_or_cursor(request, "\n".into()),
    EditIntent::DeleteBackward => delete_selection_or_adjacent_grapheme(request, doc, -1),
    EditIntent::DeleteForward => delete_selection_or_adjacent_grapheme(request, doc, 1),
    EditIntent::Indent => replace_selection_or_cursor(request, default_indent_text(doc)),
    EditIntent::Outdent => default_outdent_plan(request, doc),
}
```

`default_indent_text` 必须精确定义为：

```rust
fn default_indent_text(doc: &DocumentView) -> String {
    if doc.tb.indent_with_tabs() {
        "\t".into()
    } else {
        " ".repeat(doc.tb.tab_size() as usize)
    }
}
```

`validate_edit_transaction` 必须检查：range 顺序、range 上界、range 两端 char boundary、最终长度、cursor 上界、cursor char boundary。grapheme boundary 通过 Task 2 的 `TextBuffer::is_grapheme_boundary` 验证。

- [ ] **Step 4: 验证 App 事务规划**

Run: `cargo fmt --all -- --check && cargo test -p textora-app edit_transaction::tests`

Expected: 全部通过。

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/edit_transaction.rs crates/app/src/lib.rs
git commit -m "feat(app): plan and validate default edit transactions"
```

---

### Task 4: 原子执行 EditTransaction 并产生单一 EditOutcome

**Files:**
- Modify: `crates/app/src/edit_transaction.rs`
- Modify: `crates/app/src/commands.rs:95-173,302-344`
- Test: `crates/app/src/edit_transaction.rs` tests module

**Interfaces:**
- Consumes: Task 3 validator、`EditOutcome`、现有 ReplaceRange 行为。
- Produces: `execute_edit_plan(plan, doc, advance_cache) -> Result<EditOutcome, EditTransactionError>`。

- [ ] **Step 1: 写原子执行失败测试**

```rust
#[test]
fn execute_apply_replaces_selection_as_one_edit_and_clears_anchor() {
    let mut doc = document_from_text("hello world");
    let generation_before = doc.generation();
    let plan = EditPlan::Apply(EditTransaction {
        replacement: TextReplacement { range: 5..11, text: "\n\nnext".into() },
        cursor_after: 7,
    });

    let outcome = execute_edit_plan(plan, &mut doc, &[]).expect("valid transaction");

    assert_eq!(doc.full_text(), "hello\n\nnext");
    assert_eq!(doc.cursor_offset().to_usize(), 7);
    assert!(doc.cursor().selection_anchor.is_none());
    assert_eq!(doc.generation(), generation_before + 1);
    assert!(outcome.executed);
}

#[test]
fn execute_move_cursor_does_not_change_generation() {
    let mut doc = document_from_text("abc");
    let generation_before = doc.generation();

    let outcome = execute_edit_plan(
        EditPlan::MoveCursor(CursorUpdate { cursor_after: 2 }),
        &mut doc,
        &[],
    )
    .expect("cursor update is valid");

    assert_eq!(doc.generation(), generation_before);
    assert_eq!(doc.cursor_offset().to_usize(), 2);
    assert!(!outcome.executed);
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p textora-app execute_apply_replaces_selection_as_one_edit_and_clears_anchor execute_move_cursor_does_not_change_generation`

Expected: compile failure，executor 不存在。

- [ ] **Step 3: 提取单次替换执行原语**

把 `EditCommand::ReplaceRange` 当前分支提取为：

```rust
pub(crate) fn execute_text_replacement(
    replacement: &TextReplacement,
    cursor_after: usize,
    doc: &mut DocumentView,
) -> bool {
    let start = replacement.range.start;
    let end = replacement.range.end;
    doc.cursor_move_to_offset(end);
    doc.cursor_mut().selection_anchor = (start < end).then_some(start);
    doc.delete_selection();
    if !replacement.text.is_empty() {
        doc.insert_at_cursor(replacement.text.as_bytes());
    }
    doc.cursor_move_to_offset(cursor_after);
    doc.cursor_mut().selection_anchor = None;
    true
}
```

executor 在执行前验证事务；`Consume` 返回未执行 outcome；`UseDefault` 到达 executor 视为 `EditTransactionError::UnresolvedDefaultPlan`。

- [ ] **Step 4: 验证 executor 和原命令回归**

Run: `cargo fmt --all -- --check && cargo test -p textora-app edit_transaction && cargo test -p textora-app commands`

Expected: 全部通过；旧 ReplaceRange 测试继续通过。

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/edit_transaction.rs crates/app/src/commands.rs
git commit -m "refactor(app): execute edit transactions atomically"
```

---

### Task 5: 将 App 文本编辑入口统一路由到 EditPolicy

**Files:**
- Modify: `crates/app/src/dispatch/editor.rs:10-60,299-333,616-680`
- Modify: `crates/app/src/edit_transaction.rs`
- Test: `crates/app/src/dispatch/editor.rs` tests module

**Interfaces:**
- Consumes: `ViewPlugin::edit_policy()`、Task 3/4 functions。
- Produces: `dispatch_transactional_edit(&mut self, command, event_loop) -> AppEffect`。

- [ ] **Step 1: 写路由失败测试**

```rust
#[test]
fn delete_forward_and_tab_are_transactional_edit_intents() {
    assert_eq!(edit_intent_for_command(&EditCommand::DeleteForward), Some(EditIntent::DeleteForward));
    assert_eq!(edit_intent_for_command(&EditCommand::Tab), Some(EditIntent::Indent));
}

#[test]
fn selected_enter_still_queries_plugin_edit_policy() {
    let policy = RecordingEditPolicy::returning(EditPlan::Consume);
    let mut app = app_with_recording_policy("| a | b |", policy.clone());
    select_bytes(&mut app, 2..3);

    let effect = app.dispatch_transactional_edit_for_test(EditCommand::InsertNewline);

    assert_eq!(policy.requests().len(), 1);
    assert_eq!(policy.requests()[0].selection, Some(2..3));
    assert_eq!(app.workspace.active_doc().expect("document").full_text(), "| a | b |");
    assert!(effect.redraw);
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p textora-app delete_forward_and_tab_are_transactional_edit_intents selected_enter_still_queries_plugin_edit_policy`

Expected: selected Enter 没有调用 policy，或 helper 不存在。

- [ ] **Step 3: 实现统一路由**

`dispatch_edit_command` 在 search/widget focus 处理之后、普通 command 执行之前执行：

```rust
if let Some(intent) = edit_intent_for_command(&cmd) {
    return self.dispatch_transactional_edit(intent, event_loop);
}
```

`dispatch_transactional_edit` 固定顺序：sync 当前 plugin mirror → build request → plugin plan → resolve UseDefault → execute → invalidate cache → sync plugin once → reset preferred x → redraw。

删除当前 `selection_replacement` 对 WYSIWYG route 的短路；暂时保留旧 augmented route，但统一事务分支必须位于它之前并由 feature-local test 证明已接管目标命令。

- [ ] **Step 4: 验证 App 路由**

Run: `cargo fmt --all -- --check && cargo test -p textora-app dispatch::editor`

Expected: 新测试和现有搜索框/基础编辑器输入测试通过。

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/dispatch/editor.rs crates/app/src/edit_transaction.rs
git commit -m "refactor(app): route text edits through edit policies"
```

---

### Task 6: 建立 MarkdownEditContext 纯结构分类

**Files:**
- Create: `crates/markdown/src/edit_context.rs`
- Modify: `crates/markdown/src/lib.rs:9-19`
- Test: `crates/markdown/src/edit_context.rs` tests module

**Interfaces:**
- Consumes: pulldown-cmark offset events、`ListBullet`、静态 `SourceLineMap`。
- Produces: `MarkdownBlockContext`、`MarkdownEditContext`、`classify_markdown_edit_context(source, source_map, request)`。

- [ ] **Step 1: 写分类失败测试**

```rust
#[test]
fn classifies_cursor_on_second_trailing_empty_line_without_skipping_run_position() {
    let source = "### 基础编辑:\n\n\n";
    let map = SourceLineMap::from_source(source);
    let request = request_at(source.len() - 1, EditIntent::DeleteBackward);

    let context = classify_markdown_edit_context(source, &map, &request);

    assert_eq!(context.source_line.index, 2);
    assert_eq!(context.source_line.role, SourceLineRole::EditableEmpty);
    assert_eq!(context.empty_run_position.expect("empty run").index_in_run, 1);
}

#[test]
fn classifies_heading_interior_at_exact_cursor_byte() {
    let source = "# Ti|tle".replace('|', "");
    let request = request_at(4, EditIntent::InsertParagraphBreak);
    let map = SourceLineMap::from_source(&source);

    assert!(matches!(
        classify_markdown_edit_context(&source, &map, &request).block,
        MarkdownBlockContext::Heading { level: 1, content_range: 2..7 }
    ));
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p textora-markdown edit_context::tests`

Expected: compile failure，module/type 不存在。

- [ ] **Step 3: 实现上下文类型**

```rust
pub enum MarkdownBlockContext {
    Paragraph { content_range: Range<usize> },
    Heading { level: u8, content_range: Range<usize> },
    ListItem { marker_range: Range<usize>, content_range: Range<usize>, indent: String, bullet: ListBullet },
    BlockQuote { marker_ranges: Vec<Range<usize>>, content_range: Range<usize> },
    CodeBlock { content_range: Range<usize> },
    TableCell { content_range: Range<usize>, next_row_same_column: Option<usize> },
    Other,
}

pub struct MarkdownEditContext {
    pub source_line: SourceLineEntry,
    pub empty_run_position: Option<EmptyRunPosition>,
    pub block: MarkdownBlockContext,
    pub cursor_byte: usize,
    pub selection: Option<Range<usize>>,
}
```

分类器只能分类，不生成插入文本或 cursor_after。列表、引用、表格 frame 使用 enum 栈，禁止多个 bool 表示嵌套状态。

- [ ] **Step 4: 验证分类器**

Run: `cargo fmt --all -- --check && cargo test -p textora-markdown edit_context::tests`

Expected: paragraph、heading、list、blockquote、code、table、leading/inter-block/trailing empty 分类全通过。

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/edit_context.rs crates/markdown/src/lib.rs
git commit -m "refactor(markdown): classify structured edit context"
```

---

### Task 7: 让 SourceLineMap 成为空行角色与几何唯一来源

**Files:**
- Modify: `crates/markdown/src/layout/source_line_map.rs`
- Modify: `crates/markdown/src/layout/types.rs:833-900`
- Test: `crates/markdown/src/layout/source_line_map.rs` tests module

**Interfaces:**
- Consumes: source text、flat line source ranges、line height、paragraph spacing。
- Produces: `SourceLineRole`、`SourceLineEntry`、`SourceLineMap::attach_layout`、`SourceLineMap::extra_height_before_block`、`SourceLineMap::trailing_editable_height`。

- [ ] **Step 1: 写几何失败测试**

```rust
#[test]
fn trailing_empty_lines_are_all_editable_and_extend_content_height() {
    let source = "heading\n\n\n";
    let mut map = SourceLineMap::from_source(source);
    map.attach_layout(&single_rendered_line_layout(0..7, 0.0, 24.0), 24.0, 12.0);

    assert_eq!(map.line_at_index(1).expect("line 1").role, SourceLineRole::EditableEmpty);
    assert_eq!(map.line_at_index(2).expect("line 2").role, SourceLineRole::EditableEmpty);
    assert_eq!(map.line_at_index(3).expect("line 3").role, SourceLineRole::EditableEmpty);
    assert_eq!(map.trailing_editable_height(), 72.0);
}

#[test]
fn inter_block_run_has_one_hidden_separator_then_editable_lines() {
    let source = "a\n\n\n\nb";
    let mut map = SourceLineMap::from_source(source);
    map.attach_layout(&two_rendered_line_layout(), 24.0, 12.0);

    assert_eq!(map.line_at_index(1).expect("separator").role, SourceLineRole::HiddenBlockSeparator);
    assert_eq!(map.line_at_index(2).expect("editable").role, SourceLineRole::EditableEmpty);
    assert_eq!(map.line_at_index(3).expect("editable").role, SourceLineRole::EditableEmpty);
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p textora-markdown source_line_map::tests::trailing_empty_lines_are_all_editable_and_extend_content_height source_line_map::tests::inter_block_run_has_one_hidden_separator_then_editable_lines`

Expected: `SourceLineEntry.role` 或 attach API 不存在。

- [ ] **Step 3: 实现角色和布局附着**

`SourceLineMap` 存储 `Vec<SourceLineEntry>`；静态扫描阶段记录 range/run，attach 阶段一次性写入 role/y/height。`LazyLayout::reserve_extra_blank_source_lines` 改为读取 map 的 leading/inter-block/trailing 增量，不能再次扫描 newline 并重算公式。

角色规则：上下都有渲染块时 run 第一条为 Hidden；文档开头或末尾的所有空行均为 Editable；其余 run 条目为 Editable。

- [ ] **Step 4: 验证布局**

Run: `cargo fmt --all -- --check && cargo test -p textora-markdown source_line_map && cargo test -p textora-markdown empty_source_line`

Expected: 新旧空行几何测试通过。

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/layout/source_line_map.rs crates/markdown/src/layout/types.rs
git commit -m "refactor(markdown): centralize empty-line roles and geometry"
```

---

### Task 8: 实现 Markdown 段落、标题和空行 policy

**Files:**
- Create: `crates/markdown/src/edit_policy.rs`
- Modify: `crates/markdown/src/lib.rs:9-19`
- Test: `crates/markdown/src/edit_policy.rs` tests module

**Interfaces:**
- Consumes: Task 1 protocol、Task 6 context、Task 7 SourceLineMap。
- Produces: `MarkdownEditPolicy::new`、`MarkdownEditPolicy::update_source`、`EditPolicy::plan_edit`。

- [ ] **Step 1: 写精确复现失败测试**

```rust
#[test]
fn backspace_on_final_trailing_empty_line_deletes_one_newline_without_jumping_to_heading() {
    let source = "### 基础编辑:\n\n\n";
    let policy = policy_for(source);
    let request = request_at(source.len(), EditIntent::DeleteBackward);

    assert_eq!(
        policy.plan_edit(&request),
        EditPlan::Apply(EditTransaction {
            replacement: TextReplacement { range: source.len() - 1..source.len(), text: String::new() },
            cursor_after: source.len() - 1,
        })
    );
}

#[test]
fn repeated_backspace_removes_trailing_lines_one_at_a_time() {
    assert_backspace_sequence(
        "### 基础编辑:\n\n\n",
        &["### 基础编辑:\n\n", "### 基础编辑:\n", "### 基础编辑:"],
    );
}

#[test]
fn enter_inside_heading_splits_at_cursor() {
    let source = "# Title";
    let policy = policy_for(source);

    assert_eq!(
        policy.plan_edit(&request_at(4, EditIntent::InsertParagraphBreak)),
        apply(4..4, "\n\n", 6)
    );
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p textora-markdown edit_policy::tests::backspace_on_final_trailing_empty_line_deletes_one_newline_without_jumping_to_heading edit_policy::tests::repeated_backspace_removes_trailing_lines_one_at_a_time edit_policy::tests::enter_inside_heading_splits_at_cursor`

Expected: policy 不存在。

- [ ] **Step 3: 实现最小 policy**

段落/标题 Enter 使用当前 cursor 的空 range 插入 `\n\n`；空行 Enter 插入一个逻辑 `\n`；空行 Backspace 删除 `[previous_newline_start, cursor)` 中恰好一个 newline；空行 DeleteForward 删除 cursor 之后恰好一个 newline。CRLF 文档中 TextBuffer 内部 offset 按实际 `\r\n` 计算，replacement 必须覆盖完整 CRLF。

标题 marker 后 Backspace 只删除当前 heading marker range；标题内部 DeleteForward 不删除 marker。

- [ ] **Step 4: 验证 policy**

Run: `cargo fmt --all -- --check && cargo test -p textora-markdown edit_policy::tests`

Expected: LF、CRLF、CJK、emoji、1/2/3 trailing blank cases 全通过。

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/edit_policy.rs crates/markdown/src/lib.rs
git commit -m "feat(markdown): plan paragraph heading and empty-line edits"
```

---

### Task 9: 实现列表和引用的逐级结构事务

**Files:**
- Modify: `crates/markdown/src/edit_policy.rs`
- Modify: `crates/markdown/src/edit_context.rs`
- Test: `crates/markdown/src/edit_policy.rs` tests module

**Interfaces:**
- Consumes: list/blockquote marker ranges 和 nesting depth。
- Produces: list/blockquote Enter、Backspace、DeleteForward、Indent、Outdent plans。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn backspace_after_nested_list_marker_removes_one_indent_level() {
    let source = "  - child";
    let policy = policy_for(source);

    assert_eq!(policy.plan_edit(&request_at(4, EditIntent::DeleteBackward)), apply(0..2, "", 2));
}

#[test]
fn backspace_after_nested_quote_marker_removes_one_quote_level() {
    let source = "> > child";
    let second_marker_end = 4;
    let policy = policy_for(source);

    assert_eq!(
        policy.plan_edit(&request_at(second_marker_end, EditIntent::DeleteBackward)),
        apply(2..4, "", 2)
    );
}

#[test]
fn enter_on_nonempty_ordered_item_continues_same_level() {
    let source = "12. item";
    assert_eq!(
        policy_for(source).plan_edit(&request_at(source.len(), EditIntent::InsertParagraphBreak)),
        apply(source.len()..source.len(), "\n13. ", source.len() + 5)
    );
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p textora-markdown backspace_after_nested_list_marker_removes_one_indent_level backspace_after_nested_quote_marker_removes_one_quote_level enter_on_nonempty_ordered_item_continues_same_level`

Expected: 返回 UseDefault 或删除全部 marker。

- [ ] **Step 3: 实现逐级规则**

list context 必须暴露每级 indent range；blockquote context 必须暴露按层拆分的 marker_ranges。Backspace/Outdent 只删除最后一级 range。非空 item Enter 插入同级 marker；空 item Enter 删除当前 marker 并保留行边界；Indent 仅在存在前一个 sibling 时增加一级。

- [ ] **Step 4: 验证结构 policy**

Run: `cargo fmt --all -- --check && cargo test -p textora-markdown edit_policy::tests`

Expected: bullet、ordered、task、nested、quote tests 全通过。

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/edit_policy.rs crates/markdown/src/edit_context.rs
git commit -m "feat(markdown): plan level-aware list and quote edits"
```

---

### Task 10: 实现代码块、表格和选区保护

**Files:**
- Modify: `crates/markdown/src/edit_policy.rs`
- Modify: `crates/markdown/src/edit_context.rs`
- Test: `crates/markdown/src/edit_policy.rs` tests module

**Interfaces:**
- Consumes: code content range、table cell content/marker ranges、selection。
- Produces: structure-preserving table/code/selection plans。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn enter_with_selection_inside_table_cell_preserves_pipes() {
    let source = "| a | b |\n|---|---|\n| c | d |";
    let selection = source.find('b').expect("b cell")..source.find('b').expect("b cell") + 1;
    let request = request_with_selection(selection.clone(), EditIntent::InsertParagraphBreak);

    let plan = policy_for(source).plan_edit(&request);

    assert_eq!(plan, EditPlan::Apply(EditTransaction {
        replacement: TextReplacement { range: selection.clone(), text: String::new() },
        cursor_after: selection.start,
    }));
    assert_eq!(apply_plan_to_text(source, plan), "| a |  |\n|---|---|\n| c | d |");
}

#[test]
fn enter_in_last_table_row_is_consumed() {
    let source = "| a |\n|---|\n| b |";
    let cursor = source.find('b').expect("last cell") + 1;

    assert_eq!(
        policy_for(source).plan_edit(&request_at(cursor, EditIntent::InsertParagraphBreak)),
        EditPlan::Consume
    );
}

#[test]
fn enter_in_code_block_uses_single_logical_newline() {
    let source = "```\nfoo\n```";
    let cursor = source.find("foo").expect("code") + 2;

    assert_eq!(
        policy_for(source).plan_edit(&request_at(cursor, EditIntent::InsertParagraphBreak)),
        apply(cursor..cursor, "\n", cursor + 1)
    );
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p textora-markdown enter_with_selection_inside_table_cell_preserves_pipes enter_in_last_table_row_is_consumed enter_in_code_block_uses_single_logical_newline`

Expected: table selection回退普通 newline 或 policy 未实现。

- [ ] **Step 3: 实现虚拟源码选区规划**

非空 selection 使用：

```rust
let effective_cursor = selection.start;
let virtual_source = format!("{}{}", &source[..selection.start], &source[selection.end..]);
```

仅在选区存在时构造虚拟源码。先验证 selection 不切入受保护 marker；表格 selection 规范化到 cell content_range。最终计划仍是一个针对原 source 的连续 replacement。

- [ ] **Step 4: 验证表格、代码块和选区**

Run: `cargo fmt --all -- --check && cargo test -p textora-markdown edit_policy::tests`

Expected: pipe、separator、fence、marker 未损坏。

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/edit_policy.rs crates/markdown/src/edit_context.rs
git commit -m "feat(markdown): preserve structures during selected edits"
```

---

### Task 11: 将 MarkdownEditorView 接入 EditPolicy

**Files:**
- Modify: `crates/markdown/src/view.rs:930-943,1774-1785,2435-2455`
- Modify: `crates/markdown/src/edit_policy.rs`
- Test: `crates/markdown/src/view.rs` WYSIWYG tests

**Interfaces:**
- Consumes: `MarkdownEditPolicy`。
- Produces: `MarkdownEditorView::edit_policy()`，source update 同时更新 policy cache。

- [ ] **Step 1: 写集成失败测试**

```rust
#[test]
fn markdown_view_policy_uses_current_source_generation() {
    let mut view = MarkdownEditorView::new();
    view.set_source("first\n\n\n".into(), 3);
    let request = EditRequest {
        source_generation: 3,
        cursor_byte: "first\n\n\n".len(),
        selection: None,
        intent: EditIntent::DeleteBackward,
    };

    assert!(matches!(view.edit_policy().plan_edit(&request), EditPlan::Apply(_)));
}

#[test]
fn stale_policy_request_falls_back_without_emitting_invalid_ranges() {
    let mut view = MarkdownEditorView::new();
    view.set_source("abc".into(), 4);
    let request = EditRequest {
        source_generation: 3,
        cursor_byte: 3,
        selection: None,
        intent: EditIntent::DeleteBackward,
    };

    assert_eq!(view.edit_policy().plan_edit(&request), EditPlan::UseDefault);
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p textora-markdown markdown_view_policy_uses_current_source_generation stale_policy_request_falls_back_without_emitting_invalid_ranges`

Expected: `ViewPlugin::edit_policy` 返回 Noop。

- [ ] **Step 3: 接入 policy cache**

`MarkdownEditorView` 持有 `MarkdownEditPolicy`；`set_source` 同步 text、generation 和 SourceLineMap。`ViewPlugin::edit_policy()` 返回该实例。此任务保留旧 `augmenter()` 供兼容测试，App 新路由不能再调用它。

- [ ] **Step 4: 验证 Markdown view**

Run: `cargo fmt --all -- --check && cargo test -p textora-markdown wysiwyg_tests`

Expected: policy 集成和现有渲染测试通过。

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/view.rs crates/markdown/src/edit_policy.rs
git commit -m "feat(markdown): expose structured edit policy"
```

---

### Task 12: 统一空行光标、hit-test、导航和滚动高度

**Files:**
- Modify: `crates/markdown/src/view.rs:954-1308,1450-1772`
- Modify: `crates/markdown/src/layout/source_line_map.rs`
- Test: `crates/markdown/src/view.rs` navigation tests

**Interfaces:**
- Consumes: attached `SourceLineEntry { role, y_top, height }`。
- Produces: 所有空行视觉查询只读 SourceLineMap，连续空行按相邻行移动。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn down_moves_through_each_trailing_empty_line() {
    let source = "heading\n\n\n";
    let view = make_view(source);

    assert_eq!(view.engine().visual_move(8, MoveDirection::Down, None), Some(9));
    assert_eq!(view.engine().visual_move(9, MoveDirection::Down, None), Some(10));
}

#[test]
fn trailing_empty_cursor_is_inside_content_height() {
    let source = long_document_ending_with_three_empty_lines();
    let view = make_view(&source);
    let (_, y, _, h) = view.engine().cursor_screen_pos_for_byte(source.len()).expect("cursor rect");

    assert!(y + h + view.engine().scroll_y <= view.engine().content_height + 0.5);
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p textora-markdown down_moves_through_each_trailing_empty_line trailing_empty_cursor_is_inside_content_height`

Expected: Down 跳过相邻空行，或 cursor 超出 content_height。

- [ ] **Step 3: 删除 view 内重复空行推导**

用 `SourceLineMap::line_at_byte/previous_line/next_line` 替换 `empty_source_line_role`、`empty_source_line_metrics`、`empty_source_line_rank` 和 previous/next non-empty shortcut。Up/Down 从 EditableEmpty 优先移动到相邻 EditableEmpty；Left/Right 仅跳过 HiddenBlockSeparator。

- [ ] **Step 4: 验证几何与导航**

Run: `cargo fmt --all -- --check && cargo test -p textora-markdown empty_source_line && cargo test -p textora-markdown visual_move`

Expected: 点击、roundtrip、Home/End、方向键、scroll tests 全通过。

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/view.rs crates/markdown/src/layout/source_line_map.rs
git commit -m "refactor(markdown): unify empty-line geometry and navigation"
```

---

### Task 13: 删除 App augmented edit 递归路径

**Files:**
- Modify: `crates/app/src/dispatch/wysiwyg.rs:238-431`
- Modify: `crates/app/src/dispatch/editor.rs:10-45,299-333`
- Test: `crates/app/src/app_tests.rs:1655-1868`

**Interfaces:**
- Consumes: Task 5 unified transaction dispatch、Task 11 Markdown policy。
- Produces: WYSIWYG dispatch 只保留视觉 navigation；删除 `wysiwyg_recursing` 的文本编辑用途。

- [ ] **Step 1: 把旧 App augmentation 测试改为事务测试并确认失败**

```rust
#[test]
fn markdown_trailing_empty_backspace_executes_one_transaction() {
    let source = "### 基础编辑:\n\n\n";
    let mut app = markdown_app(source, source.len());

    let effect = app.dispatch_transactional_edit_for_test(EditCommand::Backspace);

    assert_eq!(active_text(&app), "### 基础编辑:\n\n");
    assert_eq!(active_cursor(&app), "### 基础编辑:\n\n".len());
    assert_eq!(recorded_full_sync_count(&app), 1);
    assert!(effect.redraw);
}
```

- [ ] **Step 2: 确认新测试在旧路径下失败**

Run: `cargo test -p textora-app markdown_trailing_empty_backspace_executes_one_transaction`

Expected: 一次删除多个 newline 或 sync count 不等于 1。

- [ ] **Step 3: 删除递归 augmentation 分派**

删除 `dispatch_wysiwyg_augmented_edit`、`execute_augmentation_text_change`、`wysiwyg_query_augment`、三个 augmented wrapper 和相关日志字段。`wysiwyg_route_for_command` 只保留 navigation；所有文本 intent 已由 editor.rs 的 transaction branch 处理。

若 `wysiwyg_recursing` 仅剩文本路径引用，删除字段、初始化和判断；若仍有非文本引用，重命名为准确描述该非文本职责的状态 enum。

- [ ] **Step 4: 验证 App WYSIWYG**

Run: `cargo fmt --all -- --check && cargo test -p textora-app wysiwyg`

Expected: 全部通过，`rg -n "dispatch_wysiwyg_augmented|wysiwyg_query_augment" crates/app/src` 无结果。

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/dispatch/wysiwyg.rs crates/app/src/dispatch/editor.rs crates/app/src/app_tests.rs
git commit -m "refactor(app): remove recursive WYSIWYG edit augmentation"
```

---

### Task 14A: 删除 Markdown augmenter 模块

**Files:**
- Delete: `crates/markdown/src/augmenter.rs`
- Modify: `crates/markdown/src/lib.rs:9-19`
- Modify: `crates/markdown/src/view.rs:1774-1785,2175-2177,2435-2455,2786-3208`

**Interfaces:**
- Consumes: Task 11 policy 和新测试。
- Produces: Markdown crate 不再引用 `augmenter` module 或实现 `augmenter()`。

- [ ] **Step 1: 运行符号清单作为删除前基线**

Run: `rg -n "AugmentKind|EditAugmentation|EditAugmenter|augment_edit" crates`

Expected: 仅 ui、markdown augmenter/view 和被迁移的旧测试有结果。

- [ ] **Step 2: 迁移剩余行为测试**

把旧 `augment_edit_*` 测试改为直接构造 `EditRequest` 并调用 `MarkdownEditPolicy::plan_edit`。每个测试断言完整 `EditPlan`，不能只断言 insert_text。

Run: `cargo test -p textora-markdown edit_policy::tests`

Expected: 迁移后测试通过。

- [ ] **Step 3: 删除 Markdown 旧模块**

删除 `augmenter.rs` 及 `lib.rs` module declaration；删除 view.rs re-export、`augment_edit` wrapper、`EditAugmenter` implementation 和已迁移测试。

- [ ] **Step 4: 验证删除完整性**

Run: `cargo fmt --all -- --check && cargo check -p textora-markdown -p textora-app && cargo test -p textora-markdown`

Expected: exit 0，且 `rg -n "augmenter::|augment_edit" crates/markdown` 无结果。

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/lib.rs crates/markdown/src/view.rs
git add -u crates/markdown/src/augmenter.rs
git commit -m "refactor(markdown): remove legacy augmenter module"
```

---

### Task 14B: 删除 ui augmentation 协议

**Files:**
- Modify: `crates/ui/src/plugin.rs:80-131,235,339-342`
- Test: `crates/ui/src/plugin.rs` tests module

**Interfaces:**
- Consumes: Task 14A 后 workspace 已无 augmentation consumer。
- Produces: workspace 不再包含 `AugmentKind`、`EditAugmentation`、`AugmentContext`、`EditAugmenter`、`NoopAugmenter`。

- [ ] **Step 1: 运行旧符号结构门禁并确认未满足**

Run: `rg -n "AugmentKind|EditAugmentation|AugmentContext|EditAugmenter|NoopAugmenter" crates/ui/src/plugin.rs`

Expected: 命中旧 augmentation 定义，证明结构门禁在删除前未满足。

- [ ] **Step 2: 保留新协议公共测试**

在 ui tests 中新增只使用新协议的编译测试：

```rust
#[test]
fn view_plugin_exposes_edit_policy_as_only_text_edit_extension() {
    let plugin = TestPlugin;
    let request = EditRequest {
        source_generation: 1,
        cursor_byte: 0,
        selection: None,
        intent: EditIntent::DeleteBackward,
    };

    assert_eq!(plugin.edit_policy().plan_edit(&request), EditPlan::UseDefault);
}
```

- [ ] **Step 3: 运行新协议测试**

Run: `cargo test -p textora-ui view_plugin_exposes_edit_policy_as_only_text_edit_extension`

Expected: 测试通过，证明新协议已可独立工作；删除旧类型后继续作为编译护栏。

- [ ] **Step 4: 删除 ui 旧类型**

删除五个 augmentation 类型、`PluginResponse::Augmentation` 和 `ViewPlugin::augmenter()`；保留 mindmap 使用的 `KeyInterceptor`。

- [ ] **Step 5: 验证符号完整删除**

Run: `cargo fmt --all -- --check && cargo check -p textora-ui -p textora-markdown -p textora-app && cargo test -p textora-ui`

Expected: exit 0，且 `rg -n "AugmentKind|EditAugmentation|AugmentContext|EditAugmenter|NoopAugmenter" crates` 无结果。

- [ ] **Step 6: Commit**

```bash
git add crates/ui/src/plugin.rs
git commit -m "refactor(ui): remove legacy edit augmentation protocol"
```

---

### Task 15: 收敛 EditCommand 文本修改重复实现

**Files:**
- Modify: `crates/app/src/input.rs:24-39`
- Modify: `crates/app/src/commands.rs:302-354,390-402`
- Test: `crates/app/src/edit_transaction.rs` tests module

**Interfaces:**
- Consumes: transaction dispatch 已覆盖全部文本 intent。
- Produces: `EditCommand` 保留输入语义枚举，但 `execute_edit_command` 拒绝直接执行事务型文本命令。

- [ ] **Step 1: 写边界测试**

```rust
#[test]
fn direct_command_executor_rejects_transactional_text_edits() {
    let mut doc = document_from_text("ab");
    doc.cursor_move_to_offset(2);

    let executed = execute_edit_command(&EditCommand::Backspace, &mut doc, &[]);

    assert!(!executed);
    assert_eq!(doc.full_text(), "ab");
}
```

- [ ] **Step 2: 确认测试按预期失败**

Run: `cargo test -p textora-app direct_command_executor_rejects_transactional_text_edits`

Expected: FAIL，旧 `execute_edit_command` 返回 true 并把 `ab` 改成 `a`。

- [ ] **Step 3: 删除内部 ReplaceRange/DeleteRange 命令**

删除仅供旧 augmentation 使用的 `DeleteRange`、`ReplaceRange` variants 和 commands match arms。基础 Insert/Backspace/Delete/Tab 的直接 match arms改为 `false` 并加 debug assertion，确保文本编辑只能从 transaction dispatcher 进入；测试 helper 统一调用 transaction executor。

- [ ] **Step 4: 验证命令边界**

Run: `cargo fmt --all -- --check && cargo test -p textora-app edit_transaction && cargo test -p textora-app dispatch::editor`

Expected: exit 0，`rg -n "DeleteRange|ReplaceRange" crates/app/src` 无结果。

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/input.rs crates/app/src/commands.rs crates/app/src/edit_transaction.rs
git commit -m "refactor(app): make transactions the sole text edit path"
```

---

### Task 16: 性能门槛与解析复用

**Files:**
- Modify: `crates/markdown/src/edit_policy.rs`
- Modify: `crates/markdown/src/view.rs`
- Test: `crates/markdown/src/view.rs` performance tests

**Interfaces:**
- Consumes: MarkdownEditorView 当前 generation 的 source map 和 block context cache。
- Produces: 普通 InsertText 不重新解析全文；结构 intent 复用 generation cache。

- [ ] **Step 1: 写解析次数失败测试**

```rust
#[test]
fn typing_plain_text_does_not_reparse_markdown_per_character() {
    let source = large_markdown_fixture(10_000);
    let mut view = editor_view_with_source(&source, 1);
    reset_edit_context_parse_count();

    for character in ["a", "b", "中", "c"] {
        let request = request_at(source.len(), EditIntent::InsertText(character.into()));
        let _ = view.edit_policy().plan_edit(&request);
    }

    assert_eq!(edit_context_parse_count(), 0);
}

#[test]
fn structural_edit_parses_at_most_once_per_source_generation() {
    let source = large_markdown_fixture(10_000);
    let view = editor_view_with_source(&source, 9);
    reset_edit_context_parse_count();

    let _ = view.edit_policy().plan_edit(&request_at(source.len(), EditIntent::InsertParagraphBreak));
    let _ = view.edit_policy().plan_edit(&request_at(source.len(), EditIntent::DeleteBackward));

    assert_eq!(edit_context_parse_count(), 1);
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p textora-markdown typing_plain_text_does_not_reparse_markdown_per_character structural_edit_parses_at_most_once_per_source_generation`

Expected: parse count 大于目标值。

- [ ] **Step 3: 添加 generation cache**

`MarkdownEditPolicy::update_source` 在 source generation 改变时构建 `SourceLineMap` 和 block context index；普通 InsertText 先用 source line role 与 selection 快速判定，普通正文返回 UseDefault，不触发 parser。结构 intent 查询缓存，不重新 parse。

- [ ] **Step 4: 验证性能与行为**

Run: `cargo fmt --all -- --check && cargo test -p textora-markdown typing_plain_text structural_edit_parses && cargo test -p textora-markdown edit_policy`

Expected: parse count 达标，行为测试通过。

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/edit_policy.rs crates/markdown/src/view.rs
git commit -m "perf(markdown): reuse structured edit context by generation"
```

---

### Task 17: 完整回归、手工协议与最终清理

**Files:**
- Modify: `docs/manual_test_protocol.md`
- Modify: `docs/superpowers/specs/2026-06-23-markdown-wysiwyg-editor-design.md`
- Test: workspace-wide verification

**Interfaces:**
- Consumes: 所有任务结果。
- Produces: 更新后的正式行为规范、手工回归记录和全量验证证据。

- [ ] **Step 1: 更新手工测试矩阵**

在 `docs/manual_test_protocol.md` 增加以下精确夹具：

````text
### 基础编辑:\n\n\n
paragraph\n\n\nnext
  - nested item
> > nested quote
| a | b |\n|---|---|\n| c | d |
```\ncode\n```
````

每个夹具记录 Enter、Backspace、DeleteForward、Tab、Shift+Tab、selection replacement、Undo、Redo、点击、方向键和滚动结果。

- [ ] **Step 2: 更新正式设计规范**

删除旧 augmentation API 描述，替换为本文 §2 数据流和 §1 行为表。明确 `DocumentView` 唯一真相源和单次 sync 不变量。

- [ ] **Step 3: 运行符号和洁净度检查**

```bash
rg -n "AugmentKind|EditAugmentation|EditAugmenter|dispatch_wysiwyg_augmented|wysiwyg_query_augment|wysiwyg_recursing" crates
rg -n "TODO|TBD|unwrap\(\)" crates/ui/src/plugin.rs crates/app/src/edit_transaction.rs crates/markdown/src/edit_context.rs crates/markdown/src/edit_policy.rs
```

Expected: 第一条无结果；第二条没有本次新增的 placeholder 或无理由 unwrap。

- [ ] **Step 4: 运行分层验证**

```bash
cargo fmt --all -- --check
cargo clippy -p textora-ui -p textora-markdown -p textora-app --all-targets -- -D warnings
cargo test -p textora-core
cargo test -p textora-ui
cargo test -p textora-markdown
cargo test -p textora-app
```

Expected: 全部 exit 0。

- [ ] **Step 5: 运行重大修改全面验证**

Run: `./scripts/verify.sh`

Expected: exit 0，无失败测试、clippy warning 或格式差异。

- [ ] **Step 6: Commit 文档和最终清理**

```bash
git add docs/manual_test_protocol.md docs/superpowers/specs/2026-06-23-markdown-wysiwyg-editor-design.md
git commit -m "docs: document unified edit transaction behavior"
```

---

## 5. 阶段检查点与回滚策略

| 检查点 | 合入条件 | 回滚边界 |
|---|---|---|
| A：Task 1-4 | ui 协议、core query、App executor 全绿；生产路由未切换 | 可整体回滚，不影响用户行为 |
| B：Task 5 | 基础编辑器所有文本命令走 transaction，基础编辑测试全绿 | 回滚 Task 5，保留无害协议与 executor |
| C：Task 6-10 | Markdown policy 纯函数行为矩阵全绿，尚未接 App | 回滚对应 policy task，不影响当前 WYSIWYG |
| D：Task 11-13 | Markdown 接线完成，旧 App augmented 路径删除，App+Markdown 测试全绿 | 回滚 Task 13 恢复兼容路径 |
| E：Task 14-16 | 旧协议删除，性能门槛达标 | 通过 checkpoint D branch/tag 恢复；禁止在 E 上局部复活旧 Option 协议 |
| F：Task 17 | `./scripts/verify.sh` 全绿、手工矩阵完成 | 未通过则不合并分支 |

每个 checkpoint 必须记录：commit hash、测试命令、通过数量、耗时和已知限制。禁止跨 checkpoint squash，直到最终 review 结束。

---

## 6. 验收标准

- 精确夹具 `### 基础编辑:\n\n\n` 在最后空行连续三次 Backspace，每次只减少一个 newline，cursor 始终位于被删除 range 起点。
- Enter 在段落和标题中间都从当前 cursor 分割，不再把标题中部 Enter 移到标题末尾。
- Backspace/DeleteForward 对普通段落边界和空行逐个、方向对称。
- 嵌套列表和引用一次只改变一层结构。
- 表格内 Enter、Delete、selection replacement 不破坏 pipe 和 separator。
- selection 不再绕过 plugin policy。
- 连续可编辑空行可逐行点击、上下移动、Home/End，并全部落在 content_height 内。
- 基础编辑器、Markdown WYSIWYG 均通过同一 `EditPlan -> execute_edit_plan` 文本修改入口。
- `DocumentView` 每个 Apply transaction 只增加一次 generation，并形成一条 undo history。
- 普通 InsertText 不触发 Markdown 全文解析；结构 intent 每 generation 最多构建一次上下文缓存。
- workspace 中不再存在旧 augmentation 类型和递归分派符号。
- `./scripts/verify.sh` exit 0。

---

## 7. 明确不纳入本计划的内容

- 不重写 Markdown parser、renderer、shaper 或主题系统。
- 不改变 preview-only selection 模型和 mindmap `KeyInterceptor`；`KeyInterceptor` 保留用于非文本画布结构命令。
- 不改变文件编码与保存格式策略；事务必须尊重 TextBuffer 当前 LF/CRLF 模式。
- 不实现协同编辑、操作变换或多光标。
- 不重排已有有序列表的所有 sibling；新 item 只使用当前序号 + 1。
- 不把 Markdown 结构类型泄漏到 `ui` 或 `app`。
