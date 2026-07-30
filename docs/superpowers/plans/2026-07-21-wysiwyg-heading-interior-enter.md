# WYSIWYG Heading Interior Enter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Markdown WYSIWYG 标题中部 Enter 在当前光标处分割标题，后半成为紧邻的普通段落且不产生空源码行。

**Architecture:** 保留现有 `EditAugmentation` 和编辑事务链路，只更正 `heading_enter_augmentation` 的标题中部分支。回归测试通过真实 augmentation 结果应用到源码，验证编辑位置、文本和光标三项协议。

**Tech Stack:** Rust、`textora-markdown`、内置 Rust test harness、`cargo fmt`。

## Global Constraints

- 标题中部只插入一个逻辑 `\n`。
- 后半段是普通段落，不复制标题 marker。
- 光标落在后半段正文开头。
- 标题末尾 Enter 行为保持不变。
- 不修改布局、渲染、App 分发或 UI 层协议。

---

### Task 1: 修复标题中部 Enter 编辑位置

**Files:**
- Modify: `crates/markdown/src/view.rs:3552-3561`
- Modify: `crates/markdown/src/augmenter.rs:313-336`

**Interfaces:**
- Consumes: `heading_enter_augmentation(source: &str, current_byte: usize, at_end: bool) -> Option<ui::plugin::EditAugmentation>`。
- Produces: 标题中部的 `EditAugmentation`，其 `replace_range` 为 `Some(current_byte..current_byte)`、`insert_text` 为 `Some("\n")`、`cursor_byte_after` 为 `current_byte + 1`。

- [ ] **Step 1: 写失败的回归测试**

将现有测试改为验证当前光标处分割以及最终源码：

```rust
#[test]
fn augment_edit_heading_middle_splits_at_cursor_without_empty_line() {
    let source = "# hello world";
    let cursor_byte = 4;
    let mut view = make_view(source);
    view.engine_mut().handle_set_cursor_byte(cursor_byte); // "# he|llo world"

    let augmentation = view.engine().augment_edit(cursor_byte, AugmentKind::Enter).unwrap();

    assert_eq!(augmentation.insert_text.as_deref(), Some("\n"));
    assert_eq!(augmentation.replace_range, Some(cursor_byte..cursor_byte));
    assert_eq!(augmentation.cursor_byte_after, cursor_byte + 1);

    let mut edited_source = source.to_owned();
    let replace_range = augmentation
        .replace_range
        .expect("heading interior Enter must edit at the current cursor");
    edited_source.replace_range(
        replace_range,
        augmentation
            .insert_text
            .as_deref()
            .expect("heading interior Enter must insert one logical newline"),
    );
    assert_eq!(edited_source, "# he\nllo world");
}
```

- [ ] **Step 2: 运行测试并确认按旧行为失败**

Run: `cargo test -p textora-markdown view::wysiwyg_tests::augment_edit_heading_middle_splits_at_cursor_without_empty_line -- --exact`

Expected: FAIL；旧实现返回 `replace_range == Some(13..13)`，而测试期望 `Some(4..4)`。

- [ ] **Step 3: 实施最小修复**

保留 `at_end` 分支不变，将标题中部分支改为：

```rust
let insertion = String::from("\n");
let aug = EditAugmentation {
    insert_text: Some(insertion.clone()),
    replace_range: Some(current_byte..current_byte),
    cursor_byte_after: current_byte + insertion.len(),
};
debug_assert_augmentation(&aug, source);
Some(aug)
```

- [ ] **Step 4: 运行目标测试并确认通过**

Run: `cargo test -p textora-markdown view::wysiwyg_tests::augment_edit_heading_middle_splits_at_cursor_without_empty_line -- --exact`

Expected: PASS，1 passed、0 failed。

- [ ] **Step 5: 运行 Markdown WYSIWYG 回归测试**

Run: `cargo test -p textora-markdown wysiwyg_tests`

Expected: 所有 `wysiwyg_tests` 通过，无失败。

- [ ] **Step 6: 格式与编译检查**

Run: `cargo fmt --all -- --check`

Expected: exit code 0，无格式差异。

Run: `cargo check -p textora-markdown`

Expected: exit code 0。

- [ ] **Step 7: 提交修复**

```bash
git add crates/markdown/src/augmenter.rs crates/markdown/src/view.rs
git commit -m "fix(markdown): split heading at interior enter"
```
