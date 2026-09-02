# WYSIWYG 换行完整性修复 实施方案

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修掉 WYSIWYG 编辑中会写坏 Markdown 源码语义的 4 个换行缺陷，并把编辑分类器与渲染器统一到同一份解析选项上。

**Architecture:** 全部改动收敛在 `crates/markdown` 内。缺陷修复集中在 `augmenter.rs`：新增两个硬换行标记识别工具函数，为 `EnterContext` 增加 `SetextHeading` 与 `IndentedCodeBlock` 两个变体，并新增一个「在指定位置建立块边界」的 `emit_*` 原语。解析选项统一通过在 `parser.rs` 暴露单一 `markdown_options()`，替换掉三处各自写死的 `Options::all()`。不触碰布局、投影、hit-test 与 app 层。

**Tech Stack:** Rust 2024 edition、pulldown-cmark 0.13、`unicode-segmentation`。测试全部是 `augmenter.rs` 内的单元测试，走 `augment_edit` 公开入口。

## Global Constraints

- 全程中文注释与提交信息。
- 每个任务结束前 `cargo test -p textora-markdown` 必须全绿；提交前必须编译通过。
- 单任务改动文件数不超过 3 个（本方案每个任务实际为 1–2 个文件）。
- 遵守 `cargo fmt`；`cargo clippy --workspace --all-targets -- -D warnings` 不得新增告警。
- 禁止魔法值：硬换行的「≥2 个空格」必须提取为语义化常量。
- 优先 Early Return，禁止深层 `if-else` 嵌套。
- 互斥状态用 `enum` 表达，不得用多个 `bool` 字段组合。
- 禁止裸 `.unwrap()`；确信不会 panic 时用 `.expect("详细理由")`。
- 全部任务完成后跑一次 `./scripts/verify.sh`。
- 所有新增 `EditAugmentation` 必须经过既有的 `debug_assert_augmentation`。

---

## 背景与证据

本方案针对的缺陷全部由实测确认（调用 `augmenter::augment_edit` 与 `builder::MarkdownDoc::build` 验证），编号沿用审查结论：

| 编号 | 输入（⏎ = 源码换行，␠ = 源码空格） | 当前实际结果 | 问题 |
|---|---|---|---|
| D1 | `first\⏎second`，光标在 `second` 前退格 | `first\second` | 反斜杠残留在正文，渲染成字面 `\second` |
| D2 | `first␠␠⏎second`，光标在 `first` 后 Enter | `first␠␠⏎⏎⏎second` | 硬换行标记被遗弃在段尾，且多出一个空段 |
| D4 | `Title⏎=====⏎para`，光标在 `Title` 末 Enter | `Title⏎⏎=====⏎para` | Setext 标题降级为「Title」+「=====」两个段落 |
| D5 | `␠␠␠␠let x = 1;` 末尾 Enter 后输入 `y` | `␠␠␠␠let x = 1;⏎⏎y` | 缩进代码块提前结束，`y` 变成普通段落 |
| D7 | `\| a \|⏎\|---\|⏎\| b \|` 首格 Enter | 光标停在 `\|` 之后、内容前导空格之前 | 落点不在单元格内容起点 |

根因 A：编辑分类用 `pulldown_cmark::Options::all()`，渲染用 `parse_markdown` 里写死的 5 项子集。实测 `term⏎: definition` 时渲染侧得到一个普通段落，分类侧因启用 definition list 而对所有字节返回 `Other`，Enter 退回源码编辑器语义。

D4 与 D5 的共同机制是 `augmenter.rs:469` 的 `EnterContext::CodeBlock | EnterContext::Other => None`：这两类上下文直接放弃增强，回落到「插入单个 `\n`」的默认计划。

## 非目标

以下问题**不在本方案范围**，原因逐条说明，避免实施者顺手改动：

- **D3 `<br>` 渲染成字面文本**（`builder.rs` 把 `InlineHtml` 当纯文本 `push_text`）。改成硬换行需要同步调整 `ProjectedText` 的 span 与 boundary 生成，属渲染与投影层改动，另立方案。
- **D6 表格末行 Enter 是死操作**（`next_cell_start` 为 `None` 时无兜底）。追加表格行是功能新增，且需要先确认 `TagEnd::TableRow` 的 range 是否含行尾换行，另立方案。
- **D8 段落拆分把边界空格留给某一侧**（`left␠right` 拆分后新段带前导空格）。修剪边界空白会破坏既有可逆性测试 `enter_then_backspace_restores_plain_paragraph_and_heading_cases`，与规范「实现约束 6」冲突，需要产品先决定 Enter 是否必须逐字节可逆。
- **Shift+Enter / HardBreak 编辑入口**（`EditIntent` 无 `InsertLineBreak`）。跨 `ui` / `appkit-shell` / `markdown` 三层，且需要先定义硬换行的可见提示，另立方案。
- **块类型邻接表重构**（`SourceLineMap::attach_layout` 对非空行写 dummy `SourceLineRole::Paragraph`）。属架构级重构，须先出 spec。
- **`edit_context.rs` 的存废**。`classify_markdown_edit_context` 除本模块测试外无任何调用方，是一份未接入的并行块分类实现（467 行）。本方案只统一它的解析选项，不删除；是否删除请在邻接表 spec 中一并决定。

## File Structure

| 文件 | 职责 | 本方案的改动 |
|---|---|---|
| `crates/markdown/src/parser.rs` | pulldown-cmark 包装、事件与 range 采集 | 新增 `pub fn markdown_options() -> Options`，成为解析选项唯一来源 |
| `crates/markdown/src/augmenter.rs` | Enter / Backspace / InsertText 的 Markdown 感知增强 | 新增 2 个硬换行工具函数、1 个 `emit_*` 原语、2 个 `EnterContext` 变体及其处理分支；改用 `markdown_options()` |
| `crates/markdown/src/edit_context.rs` | 未接入的并行块分类实现 | 仅把 `Options::all()` 换成 `markdown_options()`（2 处） |

任务顺序有依赖：Task 1 产出的 `hard_break_marker_ending_at` 与 `HARD_BREAK_MIN_SPACES` 被 Task 2 复用；Task 3 产出的 `emit_block_break_at` 独立。Task 6 最后做，因为它会改变分类结果，放在最后可以让前面的回归测试先锁定行为。

---

### Task 1: 退格跨硬换行时连标记一并删除（D1）

**Files:**
- Modify: `crates/markdown/src/augmenter.rs`（`backspace_paragraph_boundary` 的段落/标题分支，约 314-325 行）
- Test: `crates/markdown/src/augmenter.rs`（`mod tests`）

**Interfaces:**
- Produces: `const HARD_BREAK_MIN_SPACES: usize = 2;`
- Produces: `fn hard_break_marker_ending_at(source: &str, content_end: usize) -> Option<std::ops::Range<usize>>` — 返回紧邻 `content_end` 结束的硬换行标记范围，Task 2 复用其常量与形态定义。

- [ ] **Step 1: 写失败测试**

在 `crates/markdown/src/augmenter.rs` 的 `mod tests` 内，紧接 `backspace_at_soft_line_start_removes_single_source_newline` 之后加入：

```rust
    #[test]
    fn backspace_after_backslash_hard_break_removes_the_marker() {
        let source = "first\\\nsecond";
        let current_byte = "first\\\n".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace after a backslash hard break should join both visual lines");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "firstsecond");
        assert_eq!(augmentation.cursor_byte_after, "first".len());
    }

    #[test]
    fn backspace_after_double_space_hard_break_removes_the_marker() {
        let source = "first  \nsecond";
        let current_byte = "first  \n".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace after a double-space hard break should join both visual lines");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "firstsecond");
        assert_eq!(augmentation.cursor_byte_after, "first".len());
    }

    #[test]
    fn backspace_after_crlf_hard_break_removes_the_complete_marker_and_sequence() {
        let source = "first  \r\nsecond";
        let current_byte = "first  \r\n".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace after a CRLF hard break should join both visual lines");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "firstsecond");
        assert_eq!(augmentation.cursor_byte_after, "first".len());
    }

    #[test]
    fn backspace_keeps_a_single_trailing_space_that_is_not_a_hard_break() {
        let source = "first \nsecond";
        let current_byte = "first \n".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace at a soft line start should still join both visual lines");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "first second");
        assert_eq!(augmentation.cursor_byte_after, "first ".len());
    }

    #[test]
    fn backspace_keeps_an_escaped_backslash_that_is_not_a_hard_break() {
        // 两个连续反斜杠是转义后的字面反斜杠，其后的换行仍是软换行。
        let source = "first\\\\\nsecond";
        let current_byte = "first\\\\\n".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace at a soft line start should still join both visual lines");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "first\\\\second");
        assert_eq!(augmentation.cursor_byte_after, "first\\\\".len());
    }
```

- [ ] **Step 2: 运行测试，确认前三个失败**

Run: `cargo test -p textora-markdown --lib augmenter::tests::backspace_after`

Expected: `backspace_after_backslash_hard_break_removes_the_marker` FAIL（实得 `"first\\second"`）、`backspace_after_double_space_hard_break_removes_the_marker` FAIL（实得 `"first  second"`）、`backspace_after_crlf_hard_break_removes_the_complete_marker_and_sequence` FAIL（实得 `"first  second"`）。后两个「不是硬换行」的测试应当已经通过。

- [ ] **Step 3: 加入常量与标记识别函数**

在 `crates/markdown/src/augmenter.rs` 顶部常量区，紧接 `const MAX_LEADING_BLOCK_INDENT: usize = 3;` 之后加入：

```rust
/// CommonMark 硬换行的空格形式所需的最少行尾空格数。
const HARD_BREAK_MIN_SPACES: usize = 2;
```

在 `newline_sequence_width_at` 之后加入：

```rust
/// 紧邻 `content_end` 结束的 Markdown 硬换行标记范围。
///
/// 两种形态：行尾 `\` 或行尾 ≥[`HARD_BREAK_MIN_SPACES`] 个空格。反斜杠形式要求
/// 行尾反斜杠总数为奇数——偶数个是转义后的字面反斜杠，其后的换行仍是软换行。
fn hard_break_marker_ending_at(
    source: &str,
    content_end: usize,
) -> Option<std::ops::Range<usize>> {
    let prefix = source.as_bytes().get(..content_end)?;
    let backslashes = prefix.iter().rev().take_while(|byte| **byte == b'\\').count();
    if backslashes % 2 == 1 {
        return Some(content_end - 1..content_end);
    }
    let spaces = prefix.iter().rev().take_while(|byte| **byte == b' ').count();
    (spaces >= HARD_BREAK_MIN_SPACES).then(|| content_end - spaces..content_end)
}
```

- [ ] **Step 4: 让段落边界退格吃掉标记**

把 `backspace_paragraph_boundary` 中的段落/标题分支替换为：

```rust
        EnterContext::TopLevelParagraphEnd
        | EnterContext::ParagraphInterior
        | EnterContext::Heading { .. } => {
            // 硬换行标记随它的换行一起删除，否则会在正文里留下字面空格或反斜杠。
            let delete_start = hard_break_marker_ending_at(source, boundary_start)
                .map_or(boundary_start, |marker| marker.start);
            let aug = EditAugmentation {
                insert_text: Some(String::new()),
                replace_range: Some(delete_start..current_byte),
                cursor_byte_after: delete_start,
            };
            debug_assert_augmentation(&aug, source);
            Some(aug)
        }
```

- [ ] **Step 5: 运行测试确认通过，并确认既有测试未回归**

Run: `cargo test -p textora-markdown --lib augmenter::`

Expected: 全部 PASS。特别确认 `backspace_at_soft_line_start_removes_single_source_newline`、`backspace_at_crlf_paragraph_start_removes_complete_boundary`、`enter_then_backspace_restores_plain_paragraph_and_heading_cases` 仍然通过。

- [ ] **Step 6: 提交**

```bash
cargo fmt --all
git add crates/markdown/src/augmenter.rs
git commit -m "fix(markdown): 退格跨硬换行时连同标记一并删除

行尾反斜杠或 ≥2 个空格构成的硬换行，退格合并两个视觉行时只删了换行，
把标记字符留在正文里（反斜杠会渲染成字面 \\second）。"
```

---

### Task 2: Enter 在硬换行边界升级为块边界（D2）

**Files:**
- Modify: `crates/markdown/src/augmenter.rs`（新增 `emit_block_break_replacing`、`hard_break_boundary_after`；改写 `paragraph_enter_augmentation`）
- Test: `crates/markdown/src/augmenter.rs`（`mod tests`）

**Interfaces:**
- Consumes: `HARD_BREAK_MIN_SPACES`、`newline_sequence_width_at`、`preferred_newline_sequence`、`BLOCK_BOUNDARY_NEWLINE_COUNT`（均已存在或由 Task 1 引入）
- Produces: `fn hard_break_boundary_after(source: &str, current_byte: usize) -> Option<std::ops::Range<usize>>`
- Produces: `fn emit_block_break_replacing(source: &str, current_byte: usize, replaced: std::ops::Range<usize>) -> EditAugmentation`

- [ ] **Step 1: 写失败测试**

在 `mod tests` 内，紧接 `paragraph_enter_before_crlf_soft_break_preserves_crlf_line_endings` 之后加入：

```rust
    #[test]
    fn enter_at_double_space_hard_break_promotes_it_to_a_block_boundary() {
        let source = "first  \nsecond";
        let current_byte = "first".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter at a hard break should split the paragraph in two");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "first\n\nsecond");
        assert_eq!(augmentation.cursor_byte_after, "first\n\n".len());
    }

    #[test]
    fn enter_at_backslash_hard_break_promotes_it_to_a_block_boundary() {
        let source = "first\\\nsecond";
        let current_byte = "first".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter at a hard break should split the paragraph in two");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "first\n\nsecond");
        assert_eq!(augmentation.cursor_byte_after, "first\n\n".len());
    }

    #[test]
    fn enter_at_crlf_hard_break_keeps_crlf_line_endings() {
        let source = "first  \r\nsecond";
        let current_byte = "first".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter at a CRLF hard break should split the paragraph in two");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "first\r\n\r\nsecond");
        assert_eq!(augmentation.cursor_byte_after, "first\r\n\r\n".len());
    }

    #[test]
    fn enter_before_two_spaces_without_a_newline_is_not_a_hard_break() {
        // 段落中部的连续空格不构成硬换行，Enter 仍走普通拆段路径。
        let source = "left  right";
        let current_byte = "left".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("paragraph Enter should split the paragraph");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "left\n\n  right");
    }
```

- [ ] **Step 2: 运行测试，确认前三个失败**

Run: `cargo test -p textora-markdown --lib augmenter::tests::enter_at`

Expected: 三个 `enter_at_*` FAIL。`enter_at_double_space_hard_break_promotes_it_to_a_block_boundary` 实得 `"first  \n\n\nsecond"`。

- [ ] **Step 3: 加入 `hard_break_boundary_after`**

在 Task 1 新增的 `hard_break_marker_ending_at` 之后加入：

```rust
/// 光标正停在硬换行标记之前时，返回「标记 + 紧随其后的换行」的完整范围。
///
/// 视觉行末尾的光标落在标记之前（标记本身不参与投影），因此判定方向与
/// [`hard_break_marker_ending_at`] 相反。只识别恰好一个反斜杠的形态：多个连续
/// 反斜杠属于转义，返回 `None` 交回通用拆段分支。
fn hard_break_boundary_after(
    source: &str,
    current_byte: usize,
) -> Option<std::ops::Range<usize>> {
    let suffix = source.as_bytes().get(current_byte..)?;
    let marker_width = match suffix.iter().take_while(|byte| **byte == b'\\').count() {
        1 => 1,
        0 => {
            let spaces = suffix.iter().take_while(|byte| **byte == b' ').count();
            if spaces < HARD_BREAK_MIN_SPACES {
                return None;
            }
            spaces
        }
        _ => return None,
    };
    let newline_start = current_byte + marker_width;
    let newline_width = newline_sequence_width_at(source, newline_start)?;
    Some(current_byte..newline_start + newline_width)
}
```

- [ ] **Step 4: 加入 `emit_block_break_replacing` 原语**

在 `emit_block_break` 之后加入：

```rust
/// 用一个块边界（`\n\n`）替换 `replaced` 区间，光标落在新块起点。
///
/// 用于把源码中已经存在的硬换行边界升级为块边界：旧的标记与换行被整体消耗，
/// 不会残留在段尾。
fn emit_block_break_replacing(
    source: &str,
    current_byte: usize,
    replaced: std::ops::Range<usize>,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, current_byte);
    let insertion = newline.repeat(BLOCK_BOUNDARY_NEWLINE_COUNT);
    let aug = EditAugmentation {
        cursor_byte_after: replaced.start + insertion.len(),
        replace_range: Some(replaced),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&aug, source);
    aug
}
```

- [ ] **Step 5: 在段落 Enter 中优先处理硬换行边界**

把 `paragraph_enter_augmentation` 替换为：

```rust
fn paragraph_enter_augmentation(source: &str, current_byte: usize) -> EditAugmentation {
    // 光标停在硬换行标记前：该视觉断行升级为块边界，标记连同换行一起被消耗，
    // 否则标记会遗留在段尾（空格）或正文中（反斜杠）。
    if let Some(boundary) = hard_break_boundary_after(source, current_byte) {
        return emit_block_break_replacing(source, current_byte, boundary);
    }
    // 段尾/段中：Typora 语义 = 切两半（`\n\n`）；跨已有 `\n` 时用
    // emit_block_break 保留光标下方的原源码行。
    if cursor_touches_source_newline(source, current_byte) {
        if newline_sequence_width_at(source, current_byte).is_some() {
            return emit_block_break(source, current_byte);
        }
        return emit_source_newline(source, current_byte);
    }
    emit_block_break(source, current_byte)
}
```

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test -p textora-markdown --lib augmenter::`

Expected: 全部 PASS。特别确认 `paragraph_enter_before_soft_break_preserves_following_source_line`（软换行前 Enter 仍产出 `\n\n`）与 `enter_then_backspace_restores_plain_paragraph_and_heading_cases` 未回归。

- [ ] **Step 7: 提交**

```bash
cargo fmt --all
git add crates/markdown/src/augmenter.rs
git commit -m "fix(markdown): Enter 在硬换行边界升级为块边界

光标停在硬换行标记前按 Enter，原先在标记后插入 \\n\\n，把标记遗弃在段尾并
多出一个可编辑空段。现在整体替换「标记 + 换行」为一个块边界。"
```

---

### Task 3: Setext 标题 Enter 重定向到下划线之后（D4）

**Files:**
- Modify: `crates/markdown/src/augmenter.rs`（`EnterContext`、`classify_heading_hit`、`enter_context_augmentation`；新增 `emit_block_break_at`、`setext_heading_enter_augmentation`）
- Test: `crates/markdown/src/augmenter.rs`（`mod tests`，含改写既有断言）

**Interfaces:**
- Consumes: `emit_block_break`、`content_end_without_trailing_newline`、`heading_source_is_atx`
- Produces: `EnterContext::SetextHeading { underline_end: usize }`
- Produces: `fn emit_block_break_at(source: &str, insert_at: usize) -> EditAugmentation` — 与 `emit_block_break` 相同，但插入点由调用方指定；Task 4 之后如需在光标之外落点可复用。

- [ ] **Step 1: 改写既有断言并写新的失败测试**

`setext_heading_is_not_classified_as_atx_heading`（约 1333 行）目前断言 setext 各位置为 `EnterContext::Other`，本任务改变了这一分类。把该测试整体替换为：

```rust
    #[test]
    fn setext_heading_is_classified_as_setext_not_atx() {
        let source = "Title\n===";
        let underline_end = source.len();

        assert!(
            matches!(
                classify_enter_context(source, "Title".len()),
                EnterContext::SetextHeading { underline_end: end } if end == underline_end
            ),
            "setext title text must not use ATX heading semantics"
        );
        assert!(
            matches!(
                classify_enter_context(source, source.len()),
                EnterContext::SetextHeading { underline_end: end } if end == underline_end
            ),
            "setext underline must not use ATX heading semantics"
        );
    }
```

并在其后加入：

```rust
    #[test]
    fn setext_heading_enter_creates_editable_paragraph_after_the_underline() {
        let source = "Title\n=====\npara";
        let current_byte = "Title".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter inside a setext heading must not rewrite the heading source");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "Title\n=====\n\n\npara");
        assert_eq!(augmentation.cursor_byte_after, "Title\n=====\n\n".len());
    }

    #[test]
    fn setext_heading_enter_from_the_underline_line_also_appends_after_it() {
        let source = "Title\n-----\npara";
        let current_byte = "Title\n-----".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter on a setext underline must not rewrite the heading source");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "Title\n-----\n\n\npara");
        assert_eq!(augmentation.cursor_byte_after, "Title\n-----\n\n".len());
    }

    #[test]
    fn setext_heading_enter_at_document_end_appends_one_newline() {
        let source = "Title\n=====\n";
        let current_byte = "Title".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter inside a trailing setext heading must not rewrite the heading source");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "Title\n=====\n\n");
        assert_eq!(augmentation.cursor_byte_after, edited_source.len());
    }
```

- [ ] **Step 2: 运行测试，确认新增测试与改写后的断言均失败**

Run: `cargo test -p textora-markdown --lib augmenter::tests::setext`

Expected: 四个测试全部 FAIL。`setext_heading_enter_creates_editable_paragraph_after_the_underline` 报 `augment_edit` 返回 `None`（panic 落在 `expect` 上）；`setext_heading_is_classified_as_setext_not_atx` 报分类仍是 `Other`。

- [ ] **Step 3: 给 `EnterContext` 增加 setext 变体**

在 `pub enum EnterContext` 中，`Heading` 之后加入：

```rust
    /// Setext 标题（`Title` + `===` / `---` 下划线）。源码重写不在当前范围内，
    /// 因此 Enter 统一重定向到下划线行之后，避免把换行插进标题构造内部。
    /// `underline_end` 是下划线行内容末端（不含行尾换行）。
    SetextHeading { underline_end: usize },
```

- [ ] **Step 4: 让分类器识别 setext**

把 `classify_heading_hit` 替换为：

```rust
fn classify_heading_hit(
    source: &str,
    current_byte: usize,
    level: u8,
    start: usize,
    range: &std::ops::Range<usize>,
) -> EnterContext {
    // pulldown-cmark 对 ATX 与 setext 标题都发 `Tag::Heading`，但下面的
    // `hash_prefix` 计算只适用于 ATX。setext 走独立分支，否则回车/退格会按
    // `# ` 前缀语义破坏标题。
    let end = content_end_without_trailing_newline(source, start..range.end);
    if !heading_source_is_atx(source, start) {
        if current_byte >= start && current_byte <= end {
            return EnterContext::SetextHeading { underline_end: end };
        }
        return EnterContext::Other;
    }
    let hash_prefix = level as usize + 1; // "# " / "## " / ...
    let content_start = start.saturating_add(hash_prefix);
    let at_end = current_byte == end;
    if current_byte >= content_start && current_byte <= end {
        EnterContext::Heading { level, at_end }
    } else {
        EnterContext::Other
    }
}
```

- [ ] **Step 5: 加入落点可指定的块边界原语与 setext 处理函数**

在 `emit_block_break_replacing`（Task 2 新增）之后加入：

```rust
/// 与 [`emit_block_break`] 相同，但插入点由调用方指定而非取自光标。
///
/// `augmentation_edit_plan` 在 `replace_range` 缺省时会退回光标位置，因此
/// 落点不等于光标时必须显式给出空区间。
fn emit_block_break_at(source: &str, insert_at: usize) -> EditAugmentation {
    let mut aug = emit_block_break(source, insert_at);
    aug.replace_range = Some(insert_at..insert_at);
    debug_assert_augmentation(&aug, source);
    aug
}
```

在 `heading_enter_augmentation` 之后加入：

```rust
/// Setext 标题内 Enter：不改写标题源码，而是在下划线行之后建立块边界。
/// 结果与 ATX 标题末尾 Enter 一致——标题保留，光标落在新的可编辑空段上。
fn setext_heading_enter_augmentation(source: &str, underline_end: usize) -> EditAugmentation {
    emit_block_break_at(source, underline_end)
}
```

- [ ] **Step 6: 接入分派**

在 `enter_context_augmentation` 的 `match` 中，`EnterContext::Heading { .. }` 分支之后加入：

```rust
        EnterContext::SetextHeading { underline_end } => {
            Some(setext_heading_enter_augmentation(source, underline_end))
        }
```

- [ ] **Step 7: 运行测试确认通过**

Run: `cargo test -p textora-markdown --lib`

Expected: 全部 PASS。特别确认 `backspace_merging_paragraph_into_unmergeable_leaf_block_is_noop`（含 `"Title\n===\nparagraph"`）仍然通过——setext 现在落进 `backspace_paragraph_boundary` 的 `_ =>` 兜底分支 `guard_unmergeable_leaf_boundary`，行为与改动前一致，退格仍被拦截为空操作。

- [ ] **Step 8: 提交**

```bash
cargo fmt --all
git add crates/markdown/src/augmenter.rs
git commit -m "fix(markdown): Setext 标题内 Enter 不再摧毁标题

原先 setext 标题分类为 Other、回落到插入单个 \\n，把 Title 与下划线拆开，
标题降级成两个普通段落。现在新增 SetextHeading 分类，Enter 重定向到下划线
行之后建立块边界，与 ATX 标题末尾 Enter 语义一致。"
```

---

### Task 4: 缩进代码块 Enter 保持缩进（D5）

**Files:**
- Modify: `crates/markdown/src/augmenter.rs`（`EnterContext`、`classify_enter_context` 的代码块分支、`enter_context_augmentation`；新增 `CodeBlockFrame`、`indented_code_block_enter_augmentation`）
- Test: `crates/markdown/src/augmenter.rs`（`mod tests`）

**Interfaces:**
- Consumes: `locate_source_line_bounds`、`preferred_newline_sequence`、`content_end_without_trailing_newline`
- Produces: `EnterContext::IndentedCodeBlock`
- Produces: `fn indented_code_block_enter_augmentation(source: &str, current_byte: usize) -> Option<EditAugmentation>`

- [ ] **Step 1: 写失败测试**

在 `mod tests` 内加入：

```rust
    #[test]
    fn indented_code_block_enter_continues_the_indent() {
        let source = "    let x = 1;";
        let current_byte = source.len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter inside an indented code block should continue the indent");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(augmentation.insert_text.as_deref(), Some("\n    "));
        assert_eq!(edited_source, "    let x = 1;\n    ");
        assert_eq!(augmentation.cursor_byte_after, edited_source.len());
    }

    #[test]
    fn indented_code_block_enter_preserves_tab_indent() {
        let source = "\tlet x = 1;";
        let current_byte = source.len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter inside a tab-indented code block should continue the indent");

        assert_eq!(augmentation.insert_text.as_deref(), Some("\n\t"));
    }

    #[test]
    fn typing_after_indented_code_block_enter_stays_inside_the_block() {
        let source = "    let x = 1;";
        let enter = augment_edit(source, source.len(), AugmentKind::Enter)
            .expect("Enter should continue the indented code block");
        let after_enter = apply_augmentation_at(source, source.len(), &enter);

        assert!(
            augment_edit(
                &after_enter,
                enter.cursor_byte_after,
                AugmentKind::InsertText(String::from("y")),
            )
            .is_none(),
            "typing on the continued code line must not be rewritten as a block separator"
        );
    }

    #[test]
    fn fenced_code_block_enter_still_falls_back_to_a_plain_newline() {
        let source = "```\nlet x = 1;\n```";
        let current_byte = "```\nlet x = 1;".len();

        assert!(
            augment_edit(source, current_byte, AugmentKind::Enter).is_none(),
            "fenced code blocks keep using the default single-newline plan"
        );
    }
```

- [ ] **Step 2: 运行测试，确认前三个失败**

Run: `cargo test -p textora-markdown --lib augmenter::tests::indented_code_block_enter augmenter::tests::typing_after_indented`

Expected: `indented_code_block_enter_continues_the_indent` 与 `indented_code_block_enter_preserves_tab_indent` FAIL（`augment_edit` 返回 `None`，panic 在 `expect`）；`typing_after_indented_code_block_enter_stays_inside_the_block` FAIL（Enter 只插入 `\n`，后续输入被判为 `EmptyBlockSeparatorLine` 并改写）。`fenced_code_block_enter_still_falls_back_to_a_plain_newline` 应已通过。

- [ ] **Step 3: 给 `EnterContext` 增加缩进代码块变体**

在 `pub enum EnterContext` 中，`CodeBlock` 之后加入：

```rust
    /// 缩进代码块。每行都依赖 ≥4 空格（或制表符）缩进，Enter 必须续上缩进，
    /// 否则新行会终止代码块。围栏代码块不受此限，仍用 [`EnterContext::CodeBlock`]。
    IndentedCodeBlock,
```

- [ ] **Step 4: 让分类器区分缩进与围栏代码块**

在 `struct TableFrame` 之后加入：

```rust
struct CodeBlockFrame {
    range: std::ops::Range<usize>,
    is_indented: bool,
}
```

把 `classify_enter_context` 内的 `use pulldown_cmark::{Event, Parser, Tag, TagEnd};` 改为：

```rust
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
```

把局部变量声明 `let mut code_block_range: Option<std::ops::Range<usize>> = None;` 改为：

```rust
    let mut code_block: Option<CodeBlockFrame> = None;
```

把两个代码块事件分支替换为：

```rust
            Event::Start(Tag::CodeBlock(kind)) => {
                mark_item_content_seen(&mut item_stack);
                code_block = Some(CodeBlockFrame {
                    range: range.clone(),
                    is_indented: matches!(kind, CodeBlockKind::Indented),
                });
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(frame) = code_block.take() {
                    // pulldown-cmark 对缩进代码块的 range 包含结尾换行
                    // （fenced 则不包含）；用去尾换行的 content_end 统一两种边界，
                    // 否则紧邻的下一块首字节会被误判为代码块。
                    let end =
                        content_end_without_trailing_newline(source, frame.range.start..range.end);
                    if current_byte >= frame.range.start && current_byte <= end {
                        return if frame.is_indented {
                            EnterContext::IndentedCodeBlock
                        } else {
                            EnterContext::CodeBlock
                        };
                    }
                }
            }
```

- [ ] **Step 5: 加入缩进续行的增强函数**

在 `heading_enter_augmentation` 之后（`setext_heading_enter_augmentation` 附近）加入：

```rust
/// 缩进代码块内 Enter：续上当前行的前导空白，否则新行失去缩进、代码块在此终止。
/// 光标位于前导空白内部时不适用，返回 `None` 交回默认计划。
fn indented_code_block_enter_augmentation(
    source: &str,
    current_byte: usize,
) -> Option<EditAugmentation> {
    let (line_start, _, _) = locate_source_line_bounds(source, current_byte)?;
    // 前导空白止于行内第一个非空白字节；换行不属于空白，因此不会越过本行。
    let indent_width = source[line_start..]
        .bytes()
        .take_while(|byte| matches!(*byte, b' ' | b'\t'))
        .count();
    if current_byte < line_start + indent_width {
        return None;
    }
    let newline = preferred_newline_sequence(source, current_byte);
    let indent = &source[line_start..line_start + indent_width];
    let insertion = format!("{newline}{indent}");
    let aug = EditAugmentation {
        cursor_byte_after: current_byte + insertion.len(),
        insert_text: Some(insertion),
        ..Default::default()
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}
```

- [ ] **Step 6: 接入分派**

把 `enter_context_augmentation` 的最后一个分支替换为：

```rust
        EnterContext::IndentedCodeBlock => {
            indented_code_block_enter_augmentation(source, current_byte)
        }
        EnterContext::CodeBlock | EnterContext::Other => None,
```

- [ ] **Step 7: 运行测试确认通过**

Run: `cargo test -p textora-markdown --lib`

Expected: 全部 PASS。特别确认：`backspace_merging_paragraph_into_unmergeable_leaf_block_is_noop`（含 `"    code\nparagraph"`，缩进代码块现在归 `IndentedCodeBlock`，仍落进 `guard_unmergeable_leaf_boundary`）、`visual_move_passes_through_blank_line_inside_active_indented_code_block`、`enter_on_blank_line_after_code_block_inserts_plain_newline` 均未回归。

- [ ] **Step 8: 提交**

```bash
cargo fmt --all
git add crates/markdown/src/augmenter.rs
git commit -m "fix(markdown): 缩进代码块内 Enter 续上缩进

原先缩进与围栏代码块共用一个分类、一律回落到插入单个 \\n，新行没有缩进，
代码块提前结束，紧接的输入变成普通段落。现在分类区分两者，缩进代码块的
Enter 续上当前行前导空白。"
```

---

### Task 5: 表格跨行跳转落到单元格内容起点（D7）

**Files:**
- Modify: `crates/markdown/src/augmenter.rs`（`classify_enter_context` 的表格落点计算；新增 `table_cell_content_start`）
- Test: `crates/markdown/src/augmenter.rs`（`mod tests`）

**Interfaces:**
- Produces: `fn table_cell_content_start(source: &str, cell: &std::ops::Range<usize>) -> usize`

- [ ] **Step 1: 写失败测试**

在 `mod tests` 内加入：

```rust
    #[test]
    fn table_enter_moves_the_cursor_to_the_next_cell_content_start() {
        let source = "| a |\n|---|\n| b |";
        let current_byte = source.find('a').expect("fixture must contain the first cell");

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter inside a table cell should move to the cell below");

        assert_eq!(augmentation.replace_range, None);
        assert_eq!(augmentation.insert_text.as_deref(), Some(""));
        assert_eq!(
            augmentation.cursor_byte_after,
            source.rfind('b').expect("fixture must contain the cell below")
        );
    }

    #[test]
    fn table_enter_into_an_empty_cell_stops_at_the_cell_end() {
        let source = "| a |\n|---|\n|   |";
        let current_byte = source.find('a').expect("fixture must contain the first cell");

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter inside a table cell should move to the cell below");

        assert_eq!(
            augmentation.cursor_byte_after,
            source.rfind('|').expect("fixture must contain a closing pipe"),
            "an empty cell has no content, so the cursor stops at its end"
        );
    }
```

- [ ] **Step 2: 运行测试，确认第一个失败**

Run: `cargo test -p textora-markdown --lib augmenter::tests::table_enter`

Expected: `table_enter_moves_the_cursor_to_the_next_cell_content_start` FAIL（实得单元格起点 13，期望内容起点 14）。

- [ ] **Step 3: 加入内容起点计算函数**

在 `locate_blockquote_line` 之前加入：

```rust
/// 单元格内容起点：跳过 `|` 之后的前导空白。空单元格没有内容，落到单元格末尾。
fn table_cell_content_start(source: &str, cell: &std::ops::Range<usize>) -> usize {
    let bytes = source.as_bytes();
    let mut content_start = cell.start;
    while content_start < cell.end && matches!(bytes.get(content_start), Some(b' ' | b'\t')) {
        content_start += 1;
    }
    content_start
}
```

- [ ] **Step 4: 在落点计算中使用它**

把 `classify_enter_context` 尾部表格命中块内的 `next_cell_start` 计算替换为：

```rust
                    let next_cell_start = t
                        .cell_ranges
                        .get(row_idx + 1)
                        .and_then(|next_row| next_row.get(col_idx))
                        .map(|next_cell| table_cell_content_start(source, next_cell));
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test -p textora-markdown --lib`

Expected: 全部 PASS。特别确认 `view.rs` 的 `table_cell_empty_insertion`（表头单行表格仍得 `None`）与 `markdown_edit_policy_maps_cursor_only_augmentation_to_move_cursor`（落点变化后仍映射为 `MoveCursor`）未回归。

- [ ] **Step 6: 提交**

```bash
cargo fmt --all
git add crates/markdown/src/augmenter.rs
git commit -m "fix(markdown): 表格 Enter 落到下一单元格的内容起点

pulldown-cmark 的 TableCell range 含单元格前导空白，直接用 range.start 会把
光标停在分隔符与内容之间，紧接的输入写错位置。"
```

---

### Task 6: 统一编辑分类与渲染的解析选项（根因 A）

**Files:**
- Modify: `crates/markdown/src/parser.rs`（提取 `markdown_options`）
- Modify: `crates/markdown/src/augmenter.rs`（`classify_enter_context` 改用共享选项）
- Modify: `crates/markdown/src/edit_context.rs`（`collect_context_frames` 与 `table_cell_context` 改用共享选项，2 处）
- Test: `crates/markdown/src/augmenter.rs`（`mod tests`）

**Interfaces:**
- Produces: `pub fn markdown_options() -> pulldown_cmark::Options` — 解析选项唯一来源，供渲染与编辑分类共用。

- [ ] **Step 1: 写失败测试**

在 `mod tests` 内加入：

```rust
    #[test]
    fn definition_list_is_classified_the_way_the_renderer_lays_it_out() {
        // 渲染侧未启用 DEFINITION_LIST，`term\n: definition` 是一个普通段落。
        // 分类器必须看到同一结构，否则 Enter 会退回源码编辑器语义。
        let source = "term\n: definition";

        assert!(
            matches!(
                classify_enter_context(source, source.len()),
                EnterContext::TopLevelParagraphEnd
            ),
            "editing classifier and renderer must share one set of parser options"
        );
    }
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p textora-markdown --lib augmenter::tests::definition_list_is_classified`

Expected: FAIL——`Options::all()` 启用了 definition list，分类结果是 `EnterContext::Other`。

- [ ] **Step 3: 在 `parser.rs` 提取共享选项**

在 `crates/markdown/src/parser.rs` 的 `parse_markdown` 之前加入：

```rust
/// Markdown 解析选项的唯一来源。
///
/// 渲染（[`parse_markdown`]）与编辑分类（`augmenter::classify_enter_context`、
/// `edit_context::collect_context_frames`）必须共用同一份，否则两侧会对同一份
/// 源码看到不同的块结构：例如启用 definition list 的一侧把 `term\n: definition`
/// 当定义列表，未启用的一侧当普通段落，Enter 行为随之分叉。
pub fn markdown_options() -> Options {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    opts.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    opts
}
```

把 `parse_markdown` 开头的选项构造替换为：

```rust
pub fn parse_markdown(src: &str) -> ParsedMarkdown {
    let parser = Parser::new_ext(src, markdown_options());
```

（删除原先 `let mut opts = Options::empty();` 到 `opts.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);` 的 6 行。）

- [ ] **Step 4: 在 `augmenter.rs` 改用共享选项**

把 `classify_enter_context` 内的 parser 构造替换为：

```rust
    let parser = Parser::new_ext(source, crate::parser::markdown_options());
```

- [ ] **Step 5: 在 `edit_context.rs` 改用共享选项**

把 `collect_context_frames` 内（约 107 行）的 parser 构造替换为：

```rust
    for (event, range) in
        Parser::new_ext(source, crate::parser::markdown_options()).into_offset_iter()
```

把同文件约 273 行 `table_cell_context` 内的 parser 构造做同样替换。

- [ ] **Step 6: 运行全量测试**

Run: `cargo test -p textora-markdown`

Expected: 全部 PASS。`pulldown_cmark_emits_heading_event_for_setext_heading` 保留 `Options::all()` 不动——它验证的是 pulldown-cmark 自身对 setext 的事件行为，不是本项目的解析配置。

- [ ] **Step 7: 提交**

```bash
cargo fmt --all
git add crates/markdown/src/parser.rs crates/markdown/src/augmenter.rs crates/markdown/src/edit_context.rs
git commit -m "refactor(markdown): 解析选项收敛为单一来源

编辑分类用 Options::all()、渲染只开 5 项子集，两侧对同一份源码看到不同的块
结构（definition list 等扩展）。提取 parser::markdown_options 供三处共用。"
```

---

### Task 7: 全量验证与回归基线

**Files:**
- 无代码改动

- [ ] **Step 1: 跑完整验证脚本**

Run: `./scripts/verify.sh`

Expected: 架构边界检查、`cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 全部通过，最后输出 `All checks passed! Baseline is trusted.`

- [ ] **Step 2: 手工复核五个缺陷场景**

在 WYSIWYG 编辑视图内逐项确认：

1. 输入 `first\` + Enter + `second`（构造反斜杠硬换行后），在 `second` 行首退格 —— 源码应变成 `firstsecond`，正文里没有残留反斜杠。
2. 同上构造双空格硬换行，光标在 `first` 后按 Enter —— 应得到两个段落，段尾没有残留空格。
3. 输入 `Title` 换行 `=====`，光标回到 `Title` 末按 Enter —— 标题样式保留，下方出现一个可编辑空段。
4. 输入 4 空格缩进的一行代码，末尾 Enter 后继续输入 —— 新行仍是代码块样式。
5. 表格首行单元格内按 Enter —— 光标落在下一行单元格的第一个字符处，直接输入不会插到分隔符后面。

- [ ] **Step 3: 提交（若有格式化残留）**

```bash
cargo fmt --all
git status --short
```

若 `git status` 为空则无需提交；否则：

```bash
git add -A
git commit -m "chore(markdown): 换行完整性修复后的格式化收尾"
```

---

## 自查结论

**覆盖核对：** D1 → Task 1；D2 → Task 2；D4 → Task 3；D5 → Task 4；D7 → Task 5；根因 A → Task 6。D3、D6、D8、Shift+Enter、邻接表重构、`edit_context.rs` 存废已在「非目标」逐条说明理由。

**类型一致性核对：** `hard_break_marker_ending_at`（Task 1 产出）被 Task 2 的注释引用；`HARD_BREAK_MIN_SPACES` 由 Task 1 定义、Task 2 使用；`emit_block_break_replacing`（Task 2）与 `emit_block_break_at`（Task 3）名称不同、语义不同——前者替换一个非空区间，后者在指定位置插入空区间；`EnterContext::SetextHeading { underline_end }` 与 `EnterContext::IndentedCodeBlock` 的字段在定义处与使用处一致。

**已知遗留（不阻塞本方案）：**

- Setext 标题内带选区按 Enter 时，`selection_augmentation_edit_plan` 要求增强的 `replace_range` 覆盖选区删除点；Task 3 的落点在下划线之后、位于删除点右侧，因此会返回 `EditPlan::UseDefault` 退回默认计划，仍可能插入单个 `\n`。带选区路径需要在后续方案里单独处理。
- `hard_break_boundary_after` 只识别恰好一个反斜杠的硬换行。`\\\` + 换行（转义反斜杠后再跟硬换行）这类形态返回 `None`，走通用拆段分支，属刻意取舍。

## 执行交接

方案已存于 `docs/plans/2026-08-19-wysiwyg-linebreak-integrity-fixes.md`。两种执行方式：

1. **Subagent-Driven（推荐）** —— 每个任务派一个新的 subagent，任务间人工复核，迭代快。
2. **Inline Execution** —— 在当前会话按 `executing-plans` 批量执行，带检查点。
