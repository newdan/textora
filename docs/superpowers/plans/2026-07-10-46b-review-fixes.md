# 46b Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 `46b08250` 中统一事务路由、原子替换、Markdown 结构分类和空行几何的已确认回归。

**Architecture:** App 继续作为事务验证与执行的唯一入口，直接复用 `TextBuffer::replace_range` 的单 Undo 原语；MarkdownEditorView 通过一个临时 `EditPolicy` 适配器复用现有 augmenter，避免两套编辑语义。Markdown 分类和 SourceLineMap 分别负责纯结构与纯几何，View 只消费完整的空白行数据。

**Tech Stack:** Rust、现有 `core::buffer::TextBuffer`、`core::unicode::CursorNav`、`DocumentView`、`pulldown-cmark`、Cargo workspace tests。

## Global Constraints

- 全程保持产品名 `textora` 和 Markdown 包名 `textora-markdown`。
- `crates/ui` 只能包含纯数据协议和 UI 抽象，不能依赖 App 状态。
- Markdown 语义只能存在于 `crates/markdown`；App 只规划、验证、执行和同步事务。
- 每个任务最多修改 3 个文件，所有行为变更严格执行 RED → GREEN → REFACTOR。
- 不新增第三方依赖；复用 TextBuffer、CursorNav、pulldown-cmark 和现有 augmenter。
- 每个阶段运行 `cargo fmt --all -- --check` 和涉及 crate 的测试；最终运行 `./scripts/verify.sh`。
- 禁止 `.unwrap()`；测试不变量使用带原因的 `.expect(...)`。

---

### Task 1: 让 App 事务真正原子且完整验证边界

**Files:**
- Modify: `crates/app/src/edit_transaction.rs`
- Test: `crates/app/src/edit_transaction.rs` tests module

**Interfaces:**
- Consumes: `DocViewMut::replace_range`、`TextBuffer::is_grapheme_boundary`、`CursorNav::goto_byte`。
- Produces: 保持 `validate_edit_transaction` 和 `execute_edit_plan` 签名不变；新增私有 `is_grapheme_boundary_in_text` 与 `validate_cursor_update`。

- [ ] **Step 1: 写原子替换失败测试**

在现有 tests module 添加并收紧测试：

```rust
#[test]
fn execute_nonempty_replacement_increments_generation_once_and_undoes_once() {
    let mut doc = document_from_text("hello world");
    let generation_before = doc.generation();
    let plan = apply(5..11, "next", 9);

    execute_edit_plan(plan, &mut doc, &[]).expect("transaction must be valid");

    assert_eq!(doc.full_text(), "hellonext");
    assert_eq!(doc.generation(), generation_before + 1);
    crate::commands::execute_edit_command_v2(&EditCommand::Undo, &mut doc, &[]);
    assert_eq!(doc.full_text(), "hello world");
}
```

- [ ] **Step 2: 写事务边界失败测试**

```rust
#[test]
fn validator_rejects_range_inside_grapheme_cluster() {
    let source = "e\u{301}x";
    assert!(validate_edit_transaction(source, &EditTransaction {
        replacement: TextReplacement { range: 1..3, text: "Q".into() },
        cursor_after: 2,
    }).is_err());
}

#[test]
fn validator_rejects_cursor_inside_inserted_utf8_character() {
    assert!(validate_edit_transaction("", &EditTransaction {
        replacement: TextReplacement { range: 0..0, text: "中".into() },
        cursor_after: 1,
    }).is_err());
}

#[test]
fn move_cursor_rejects_out_of_bounds_and_non_grapheme_positions() {
    let mut doc = document_from_text("e\u{301}x");
    assert!(execute_edit_plan(
        EditPlan::MoveCursor(CursorUpdate { cursor_after: 99 }), &mut doc, &[]
    ).is_err());
    assert!(execute_edit_plan(
        EditPlan::MoveCursor(CursorUpdate { cursor_after: 1 }), &mut doc, &[]
    ).is_err());
}
```

- [ ] **Step 3: 运行测试并确认 RED**

Run:

```bash
cargo test -p textora-app edit_transaction::tests::execute_nonempty_replacement_increments_generation_once_and_undoes_once
cargo test -p textora-app edit_transaction::tests::validator_rejects_range_inside_grapheme_cluster
cargo test -p textora-app edit_transaction::tests::move_cursor_rejects_out_of_bounds_and_non_grapheme_positions
```

Expected: generation 为 `+2`、一次 Undo 不能恢复原文，三个边界验证返回 `Ok`。

- [ ] **Step 4: 实现完整验证和单次替换**

添加边界 helper；`CursorNav` 必须在 char boundary 检查之后使用：

```rust
fn is_grapheme_boundary_in_text(text: &str, byte: usize) -> bool {
    if byte > text.len() || !text.is_char_boundary(byte) {
        return false;
    }
    let document = text.as_bytes();
    core::unicode::CursorNav::new(&document).goto_byte(core::types::ByteIndex(byte)).offset
        == core::types::ByteIndex(byte)
}

fn validate_cursor_update(source: &str, cursor_after: usize) -> Result<(), EditTransactionError> {
    if cursor_after > source.len() {
        return Err(EditTransactionError::CursorOutOfBounds {
            cursor_after,
            final_len: source.len(),
        });
    }
    if !source.is_char_boundary(cursor_after) {
        return Err(EditTransactionError::InvalidCharBoundary { byte: cursor_after });
    }
    if !is_grapheme_boundary_in_text(source, cursor_after) {
        return Err(EditTransactionError::InvalidGraphemeBoundary { byte: cursor_after });
    }
    Ok(())
}
```

给 `EditTransactionError` 增加 `InvalidGraphemeBoundary { byte }`。`validate_edit_transaction` 必须：

```rust
if !is_grapheme_boundary_in_text(source, range.start)
    || !is_grapheme_boundary_in_text(source, range.end)
{
    return Err(EditTransactionError::InvalidGraphemeBoundary {
        byte: if !is_grapheme_boundary_in_text(source, range.start) {
            range.start
        } else {
            range.end
        },
    });
}
let mut final_source = source.to_owned();
final_source.replace_range(range.clone(), &transaction.replacement.text);
validate_cursor_update(&final_source, transaction.cursor_after)?;
```

`execute_text_replacement` 改为一次 replace：

```rust
use core::document::DocViewMut as _;

doc.replace_range(replacement.range.clone(), &replacement.text);
doc.cursor_move_to_offset(cursor_after);
doc.cursor_mut().selection_anchor = None;
true
```

`EditPlan::MoveCursor` 在移动前调用 `validate_cursor_update(&doc.full_text(), update.cursor_after)`。

- [ ] **Step 5: 验证 GREEN 并提交**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-app edit_transaction::tests
cargo test -p textora-app commands::command_tests
```

Expected: 全部通过且无 warning。

Commit:

```bash
git add crates/app/src/edit_transaction.rs
git commit -m "fix(app): make edit transactions atomic"
```

---

### Task 2: 让新事务入口保留 Markdown 增强行为

**Files:**
- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/app/src/app_tests.rs`
- Test: 两文件现有 tests module

**Interfaces:**
- Consumes: `EditRequest`、`EditIntent`、`EditPlan`、现有 `PreviewEngine::augment_edit`。
- Produces: `impl EditPolicy for MarkdownEditorView`、`ViewPlugin::edit_policy()` override。

- [ ] **Step 1: 写真实 Markdown policy 路由失败测试**

在 `app_tests.rs` 使用真实 MarkdownEditorView：

```rust
#[test]
fn markdown_empty_list_enter_uses_structural_edit_policy() {
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec!["- ".to_string()], 80, 10.0);
    doc.cursor_move_to_offset(2);
    app.workspace.push_entry_for_test(DocItem::new(
        doc,
        Box::new(markdown::view::MarkdownEditorView::new()),
    ));
    let _ = app.workspace.switch_to(0);

    app.dispatch_transactional_edit_for_test(EditCommand::InsertNewline);

    assert_eq!(app.workspace.active_doc().expect("active document").full_text(), "");
}
```

在 `view.rs` tests module 添加纯转换测试，覆盖表格纯光标移动：

```rust
#[test]
fn markdown_edit_policy_maps_cursor_only_augmentation_to_move_cursor() {
    let mut view = MarkdownEditorView::new();
    let source = "| a |\n|---|\n| b |";
    view.set_source(source.into(), 1);
    let request = EditRequest {
        source_generation: 1,
        cursor_byte: source.find('a').expect("first cell"),
        selection: None,
        intent: EditIntent::InsertParagraphBreak,
    };

    assert!(matches!(view.plan_edit(&request), EditPlan::MoveCursor(_)));
}
```

- [ ] **Step 2: 运行测试并确认 RED**

Run:

```bash
cargo test -p textora-app markdown_empty_list_enter_uses_structural_edit_policy
cargo test -p textora-markdown markdown_edit_policy_maps_cursor_only_augmentation_to_move_cursor
```

Expected: 空列表结果为 `"- \n"`；MarkdownEditorView 尚未实现 `EditPolicy`。

- [ ] **Step 3: 实现兼容 policy 适配器**

在 `view.rs` 添加精确映射：

```rust
fn request_augment_kind(intent: &ui::plugin::EditIntent) -> Option<ui::plugin::AugmentKind> {
    match intent {
        ui::plugin::EditIntent::InsertText(text) => {
            Some(ui::plugin::AugmentKind::InsertText(text.clone()))
        }
        ui::plugin::EditIntent::InsertParagraphBreak => Some(ui::plugin::AugmentKind::Enter),
        ui::plugin::EditIntent::DeleteBackward => Some(ui::plugin::AugmentKind::Backspace),
        ui::plugin::EditIntent::Indent => Some(ui::plugin::AugmentKind::Tab),
        ui::plugin::EditIntent::DeleteForward | ui::plugin::EditIntent::Outdent => None,
    }
}

fn augmentation_edit_plan(
    request: &ui::plugin::EditRequest,
    augmentation: ui::plugin::EditAugmentation,
) -> ui::plugin::EditPlan {
    let range = augmentation
        .replace_range
        .unwrap_or(request.cursor_byte..request.cursor_byte);
    let text = augmentation.insert_text.unwrap_or_default();
    if range.is_empty() && text.is_empty() {
        if augmentation.cursor_byte_after == request.cursor_byte {
            return ui::plugin::EditPlan::Consume;
        }
        return ui::plugin::EditPlan::MoveCursor(ui::plugin::CursorUpdate {
            cursor_after: augmentation.cursor_byte_after,
        });
    }
    ui::plugin::EditPlan::Apply(ui::plugin::EditTransaction {
        replacement: ui::plugin::TextReplacement { range, text },
        cursor_after: augmentation.cursor_byte_after,
    })
}

impl ui::plugin::EditPolicy for MarkdownEditorView {
    fn plan_edit(&self, request: &ui::plugin::EditRequest) -> ui::plugin::EditPlan {
        if request.selection.is_some() {
            return ui::plugin::EditPlan::UseDefault;
        }
        let Some(kind) = request_augment_kind(&request.intent) else {
            return ui::plugin::EditPlan::UseDefault;
        };
        self.engine
            .augment_edit(request.cursor_byte, kind)
            .map_or(ui::plugin::EditPlan::UseDefault, |augmentation| {
                augmentation_edit_plan(request, augmentation)
            })
    }
}
```

在 `impl ViewPlugin for MarkdownEditorView` 添加：

```rust
fn edit_policy(&self) -> &dyn ui::plugin::EditPolicy {
    self
}
```

- [ ] **Step 4: 验证 GREEN 并提交**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-markdown markdown_edit_policy
cargo test -p textora-app markdown_empty_list_enter_uses_structural_edit_policy
cargo test -p textora-app selected_enter_still_queries_plugin_edit_policy
```

Commit:

```bash
git add crates/markdown/src/view.rs crates/app/src/app_tests.rs
git commit -m "fix(markdown): preserve structural edits through policy routing"
```

---

### Task 3: 修复 Markdown 结构上下文分类

**Files:**
- Modify: `crates/markdown/src/edit_context.rs`
- Modify: `crates/markdown/src/augmenter.rs`
- Test: 两文件现有 tests module

**Interfaces:**
- Consumes: pulldown-cmark offset events、共享 list marker parser、SourceLineMap。
- Produces: 正确的 `MarkdownBlockContext` 字节范围与结构元数据。

- [ ] **Step 1: 写结构分类失败测试**

在 `edit_context.rs` tests module 添加独立测试：

```rust
#[test]
fn classifies_nested_blockquote_before_inner_paragraph() {
    let source = "> > quote";
    let ctx = classify(source, 5);
    assert!(matches!(ctx.block, MarkdownBlockContext::BlockQuote {
        marker_ranges,
        content_range,
    } if marker_ranges == vec![0..2, 2..4] && content_range == (4..9)));
}

#[test]
fn classifies_ordered_and_tab_separated_list_markers() {
    let ordered = classify("42. item", 8);
    assert!(matches!(ordered.block, MarkdownBlockContext::ListItem {
        bullet: ListBullet::Ordered(42), marker_range, content_range, ..
    } if marker_range == (0..4) && content_range == (4..8)));

    let tabbed = classify("-\titem", 6);
    assert!(matches!(tabbed.block, MarkdownBlockContext::ListItem {
        marker_range, content_range, ..
    } if marker_range == (0..2) && content_range == (2..6)));
}

#[test]
fn table_cell_points_to_next_row_same_column() {
    let source = "| A | B |\n|---|---|\n| C | D |";
    let ctx = classify(source, source.find('A').expect("first cell"));
    let next = source.find('C').expect("same column next row");
    assert!(matches!(ctx.block, MarkdownBlockContext::TableCell {
        next_row_same_column: Some(byte), ..
    } if byte == next));
}
```

再添加标题、代码和空行测试：

```rust
#[test]
fn heading_content_range_keeps_inline_markers_and_empty_atx_boundary() {
    assert_heading_range("# **Title**", 5, 2..11);
    assert_heading_range("#", 1, 1..1);
    assert_heading_range("标题\n====", 3, 0..6);
}

#[test]
fn fenced_code_content_excludes_closing_fence_with_spaces_and_crlf() {
    assert_code_range("```rust\r\ncode\r\n```   \r\n", 10, 9..15);
}

#[test]
fn empty_document_and_trailing_blank_are_editable() {
    assert_eq!(classify("", 0).source_line.role, SourceLineRole::EditableEmpty);
    let source = "heading\n";
    assert_eq!(classify(source, source.len()).source_line.role, SourceLineRole::EditableEmpty);
}
```

- [ ] **Step 2: 运行测试并确认 RED**

Run:

```bash
cargo test -p textora-markdown edit_context::tests -- --nocapture
```

Expected: 引用返回 Paragraph、有序序号为 1、table next row 为 None、ATX marker 被排除、closing fence 被纳入内容、尾空行为 Hidden。

- [ ] **Step 3: 统一列表 marker parser**

在 `augmenter.rs` 把 `parse_list_marker` 和 `list_item_indent` 改为 `pub(crate)`，并让 parser 支持：

```rust
// unordered: '-', '+', '*'; ordered: 1-9 ASCII digits plus '.' or ')'
// marker separator: one ASCII space or tab
// task suffix: '[ ]', '[x]', '[X]' followed by optional space/tab
pub(crate) fn parse_list_marker(
    source: &str,
    marker_start: usize,
) -> Option<(ListBullet, usize)>;

pub(crate) fn list_item_indent(source: &str, marker_start: usize) -> String;
```

返回的第二项必须是正文起点；`42. item` 返回 `Ordered(42), 4`，`-\titem` 返回 `Bullet, 2`，`- [X] done` 返回 `TaskList(true), 6`。补充 augmenter parser 单元测试并先确认旧实现失败。

- [ ] **Step 4: 用显式 frame 和优先级重写分类决策**

`edit_context.rs` parser 扫描必须收集所有命中结构，但最终按以下固定顺序选择：

```rust
TableCell -> ListItem -> BlockQuote -> Heading -> CodeBlock -> Paragraph
```

实现以下私有纯函数并在主分类器中调用：

```rust
fn block_quote_line_ranges(source: &str, cursor: usize) -> (Vec<Range<usize>>, Range<usize>);
fn heading_content_range(source: &str, tag_range: Range<usize>) -> Range<usize>;
fn fenced_code_content_range(source: &str, tag_range: Range<usize>) -> Range<usize>;
fn table_cell_context(source: &str, cursor: usize) -> Option<(Range<usize>, Option<usize>)>;
```

空白行角色规则必须直接表达：

```rust
if source_line.is_empty() {
    source_line.role = match source_line.role {
        SourceLineRole::EditableEmpty | SourceLineRole::HiddenBlockSeparator => source_line.role,
        _ => {
            let has_previous = source_map.previous_non_empty(source_line.index).is_some();
            let has_next = source_map.next_non_empty(source_line.index).is_some();
            if has_previous
                && has_next
                && source_map.empty_run_position(source_line.index)
                    .is_some_and(|position| position.index_in_run == 0)
            {
                SourceLineRole::HiddenBlockSeparator
            } else {
                SourceLineRole::EditableEmpty
            }
        }
    };
}
```

- [ ] **Step 5: 验证 GREEN、Clippy 并提交**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-markdown edit_context::tests
cargo test -p textora-markdown augmenter
cargo clippy -p textora-markdown --all-targets -- -D warnings
```

Commit:

```bash
git add crates/markdown/src/edit_context.rs crates/markdown/src/augmenter.rs
git commit -m "fix(markdown): classify structured edit contexts precisely"
```

---

### Task 4: 修复 SourceLineMap 的软换行与半开区间几何

**Files:**
- Modify: `crates/markdown/src/layout/source_line_map.rs`
- Test: 同文件 tests module

**Interfaces:**
- Consumes: 排序的 `RenderedLineLayout` 半开源码范围。
- Produces: 每条 SourceLineEntry 的完整 `y_top`、`height` 和空行角色。

- [ ] **Step 1: 写几何失败测试**

```rust
#[test]
fn source_line_height_includes_all_soft_wrapped_segments() {
    let mut map = SourceLineMap::from_source("abcdefghij\n\nnext");
    let rendered = vec![
        RenderedLineLayout { source_range: 0..4, y_top: 0.0, height: 24.0 },
        RenderedLineLayout { source_range: 4..8, y_top: 24.0, height: 24.0 },
        RenderedLineLayout { source_range: 8..10, y_top: 48.0, height: 24.0 },
        RenderedLineLayout { source_range: 12..16, y_top: 84.0, height: 24.0 },
    ];
    map.attach_layout(&rendered, 24.0, 12.0);
    assert_eq!(map.line_at_index(0).expect("wrapped line").height, 72.0);
    assert_eq!(map.line_at_index(1).expect("separator").y_top, 72.0);
}

#[test]
fn adjacent_half_open_range_does_not_overlap_next_source_line() {
    let mut map = SourceLineMap::from_source("a\nb");
    let rendered = vec![
        RenderedLineLayout { source_range: 0..2, y_top: 0.0, height: 24.0 },
        RenderedLineLayout { source_range: 2..3, y_top: 24.0, height: 24.0 },
    ];
    map.attach_layout(&rendered, 24.0, 12.0);
    assert_eq!(map.line_at_index(1).expect("second line").y_top, 24.0);
}
```

- [ ] **Step 2: 运行测试并确认 RED**

Run:

```bash
cargo test -p textora-markdown layout::source_line_map::tests -- --nocapture
```

Expected: wrapped height 为 24、separator y 为 24、第二行错误继承 y=0。

- [ ] **Step 3: 聚合全部重叠渲染段**

在 `attach_layout` 中使用半开区间：

```rust
while rendered_idx < rendered_lines.len()
    && rendered_lines[rendered_idx].source_range.end <= line.start
{
    rendered_idx += 1;
}

let first_rendered_idx = rendered_idx;
while rendered_idx < rendered_lines.len()
    && rendered_lines[rendered_idx].source_range.start < line.end
    && rendered_lines[rendered_idx].source_range.end > line.start
{
    rendered_idx += 1;
}

if first_rendered_idx < rendered_idx {
    let first = &rendered_lines[first_rendered_idx];
    let last = &rendered_lines[rendered_idx - 1];
    line_y = first.y_top;
    line_h = (last.y_top + last.height - line_y).max(first.height);
    is_rendered = true;
    prev_had_block = true;
}
```

空行分支只推进已满足 `end <= line.start` 的旧段，不能消费下一块。

- [ ] **Step 4: 验证 GREEN 并提交**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-markdown layout::source_line_map::tests
```

Commit:

```bash
git add crates/markdown/src/layout/source_line_map.rs
git commit -m "fix(markdown): aggregate source line layout geometry"
```

---

### Task 5: 让 View 完整保留 Unicode 空白行语义

**Files:**
- Modify: `crates/markdown/src/view.rs`
- Test: 同文件 WYSIWYG tests module

**Interfaces:**
- Consumes: `SourceLineEntry.is_blank`。
- Produces: 带 `is_blank` 的 `SourceLineAtByte`，统一供 cursor、hit-test、navigation 使用。

- [ ] **Step 1: 写空白行交互失败测试**

在现有 WYSIWYG tests module 添加：

```rust
#[test]
fn whitespace_and_crlf_blank_lines_keep_empty_line_cursor_and_navigation() {
    use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

    for (source, blank_byte) in [("first\n \n", 6), ("first\r\n\r\n", 7)] {
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        view.handle_message(PluginMessage::SetCursorByte(blank_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        assert!(view.engine().cursor_screen_pos().is_some());
        assert_eq!(
            view.engine().visual_move(blank_byte, MoveDirection::LineStart, None),
            Some(blank_byte),
        );
    }
}

#[test]
fn whitespace_only_block_separator_is_not_clickable_as_editable_line() {
    let source = "first\n \nsecond";
    let doc = StubDoc::new(source);
    let mut view = MarkdownEditorView::new();
    view.set_source(source.to_owned(), 1);
    render_editor_once(&mut view, &doc);
    let first = view.engine().flat_lines().iter()
        .find(|line| line.text.contains("first"))
        .expect("first paragraph must be rendered");
    let second = view.engine().flat_lines().iter()
        .find(|line| line.text.contains("second"))
        .expect("second paragraph must be rendered");
    let separator_y = (first.rect.y + first.rect.h + second.rect.y) * 0.5;

    assert_ne!(
        view.engine().hit_test_byte(first.rect.x, separator_y, 0.0, 0.0),
        Some(6),
    );
}
```

- [ ] **Step 2: 运行测试并确认 RED**

Run:

```bash
cargo test -p textora-markdown whitespace_and_crlf_blank_lines -- --nocapture
cargo test -p textora-markdown whitespace_only_block_separator -- --nocapture
```

Expected: cursor position / Down 返回 None，空白 separator 被命中为 byte 6。

- [ ] **Step 3: 保留 is_blank 并统一构造**

修改桥接类型：

```rust
struct SourceLineAtByte {
    index: usize,
    start: usize,
    end: usize,
    is_blank: bool,
}

impl SourceLineAtByte {
    fn is_empty(self) -> bool {
        self.is_blank
    }
}
```

缓存转换复制字段：

```rust
SourceLineAtByte {
    index: entry.index,
    start: entry.start,
    end: entry.end,
    is_blank: entry.is_blank,
}
```

扫描构造必须按完整行内容计算：

```rust
let is_blank = source[line_start..line_end].chars().all(char::is_whitespace);
Some(SourceLineAtByte { index: line_index, start: line_start, end: line_end, is_blank })
```

- [ ] **Step 4: 验证 GREEN 并提交**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-markdown whitespace_and_crlf_blank_lines
cargo test -p textora-markdown whitespace_only_block_separator
cargo test -p textora-markdown empty_source_line
```

Commit:

```bash
git add crates/markdown/src/view.rs
git commit -m "fix(markdown): preserve blank line semantics in view mapping"
```

---

### Task 6: 全量验证与最终审查

**Files:**
- No production file changes expected.

**Interfaces:**
- Consumes: Tasks 1-5 的提交。
- Produces: 可合入的验证证据与最终代码审查结论。

- [ ] **Step 1: 运行局部回归集合**

```bash
cargo test -p textora-app edit_transaction
cargo test -p textora-app markdown_empty_list_enter_uses_structural_edit_policy
cargo test -p textora-markdown edit_context
cargo test -p textora-markdown source_line_map
cargo test -p textora-markdown empty_source_line
```

Expected: 全部 exit 0。

- [ ] **Step 2: 运行仓库强制验证**

```bash
./scripts/verify.sh
```

Expected: fmt、workspace Clippy `-D warnings`、workspace tests 全部 exit 0。

- [ ] **Step 3: 检查工作树和提交范围**

```bash
git status --short
git diff 46b08250..HEAD --check
git log --oneline 46b08250..HEAD
```

Expected: 工作树干净；无 whitespace error；提交只覆盖设计文档、计划和五个修复任务。

- [ ] **Step 4: 发起最终代码审查**

审查范围使用 `46b08250..HEAD`，重点复核：单 Undo/generation、真实 Markdown 路由、所有事务边界、结构范围、CRLF/Unicode 空白和软换行几何。Critical/Important 问题必须修复并重新审查。
