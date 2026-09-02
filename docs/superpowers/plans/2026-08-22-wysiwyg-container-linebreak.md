# WYSIWYG 容器换行修复实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让列表项和引用行的硬换行 Enter/Backspace 与顶层段落共用同一套标记识别，修正容器内已有换行上 Enter 的重复插入，拆段时吃掉边界空格，并为表格末行提供新增行/退出。

**Architecture:** 不新增编辑 intent。在 `augmenter.rs` 把已有 `hard_break_boundary_after` / `hard_break_marker_ending_at` 接到 `list_item_enter_augmentation` 与 `blockquote_enter_augmentation`；新增与 `emit_block_break_replacing` 对称的 `emit_marker_break_replacing`；Backspace 在 marker 删除之前合并硬换行视觉行。表格分类携带末行元数据，末行 Enter 写源码而不是空跳转。

**Tech Stack:** Rust、pulldown-cmark 0.13、`textora-markdown` 库测试、既有 `apply_augmentation_at` 辅助函数。

## Global Constraints

- 生产实现只改 `crates/markdown`（本计划即 `augmenter.rs` 与 2026-08-02 规范补丁）；不改 UI、app 生产逻辑、投影或 hit-test。
- 硬换行反斜杠奇偶性、最少两个行尾空格、LF/CRLF 规则与 2026-08-19 计划完全一致，禁止再写一份识别函数。
- 不引入 `AugmentKind` 新变体，不实现 Shift+Enter。
- 每个任务先写失败测试再改生产代码；`cargo fmt --all` 后提交。
- 使用语义化命名，测试里不用 `.unwrap()`，用 `expect("...")`。

## File map

- Modify: `crates/markdown/src/augmenter.rs` — 全部生产逻辑与单测
- Modify: `docs/specs/2026-08-02-markdown-wysiwyg-enter-backspace-behavior.md` — 补列表/引用/表格/空格矩阵（Task 6）
- Spec: `docs/superpowers/specs/2026-08-22-wysiwyg-container-linebreak-design.md`

---

### Task 1: 列表/引用硬换行上的 Enter 替换标记

**Files:**
- Modify: `crates/markdown/src/augmenter.rs`（`emit_marker_break` 附近新增 `emit_marker_break_replacing`；改 `list_item_enter_augmentation` / `blockquote_enter_augmentation`）
- Test: 同文件 `mod tests`

**Interfaces:**
- Consumes: 已有 `hard_break_boundary_after(source, current_byte) -> Option<Range<usize>>`、`preferred_newline_sequence`、`emit_marker_break`
- Produces: `fn emit_marker_break_replacing(source: &str, replaced: std::ops::Range<usize>, indent: &str, marker: &str) -> EditAugmentation`，光标落在 `replaced.start + insertion.len()`

- [ ] **Step 1: 写失败测试**

把下列测试追加到 `augmenter.rs` 的 `mod tests`，紧跟既有 `enter_at_backslash_hard_break_promotes_it_to_a_block_boundary`：

```rust
#[test]
fn list_enter_at_backslash_hard_break_promotes_it_to_a_new_item() {
    let source = "- first\\\n  second";
    let current_byte = "- first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter at a list hard break should split into two items");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- first\n- second");
    assert_eq!(augmentation.cursor_byte_after, "- first\n- ".len());
}

#[test]
fn list_enter_at_odd_backslash_hard_break_preserves_escaped_backslashes() {
    let source = "- first\\\\\\\n  second";
    let current_byte = "- first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter at an odd backslash run should keep escaped backslashes");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- first\\\\\n- second");
}

#[test]
fn list_enter_at_double_space_hard_break_promotes_it_to_a_new_item() {
    let source = "- first  \n  second";
    let current_byte = "- first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter at a double-space list hard break should split into two items");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- first\n- second");
}

#[test]
fn quote_enter_at_backslash_hard_break_promotes_it_to_an_explicit_line() {
    let source = "> first\\\n> second";
    let current_byte = "> first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter at a quote hard break should continue with an explicit marker");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> first\n> second");
    assert_eq!(augmentation.cursor_byte_after, "> first\n> ".len());
}

#[test]
fn quote_enter_at_crlf_hard_break_keeps_crlf_line_endings() {
    let source = "> first\\\r\n> second";
    let current_byte = "> first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter at a CRLF quote hard break should keep CRLF");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> first\r\n> second");
}

#[test]
fn list_enter_before_even_backslashes_does_not_treat_them_as_hard_break() {
    let source = "- first\\\\\n  second";
    let current_byte = "- first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("even backslashes are not a hard break");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_ne!(edited_source, "- first\n- second");
    assert!(
        edited_source.contains('\\'),
        "escaped backslashes must remain: {edited_source:?}"
    );
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-markdown --lib list_enter_at_backslash_hard_break_promotes_it_to_a_new_item quote_enter_at_backslash_hard_break_promotes_it_to_an_explicit_line`

Expected: FAIL，实际源码类似 `"- first\n- \\\n  second"` / `"> first\n> \\\n> second"`（标记被留给下一行）。

- [ ] **Step 3: 实现替换型 marker break 并接到容器 Enter**

在 `emit_marker_break` 之后新增：

```rust
fn emit_marker_break_replacing(
    source: &str,
    replaced: std::ops::Range<usize>,
    indent: &str,
    marker: &str,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, replaced.start);
    let insertion = format!("{newline}{indent}{marker}");
    let aug = EditAugmentation {
        cursor_byte_after: replaced.start + insertion.len(),
        replace_range: Some(replaced),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&aug, source);
    aug
}
```

改 `list_item_enter_augmentation`：在 `empty` 分支之后、`emit_marker_break` 之前：

```rust
if let Some(boundary) = hard_break_boundary_after(source, current_byte) {
    return Some(emit_marker_break_replacing(
        source,
        boundary,
        indent,
        continuation_marker,
    ));
}
```

`blockquote_enter_augmentation` 同样，`indent` 传 `""`，`marker` 传 `continuation_prefix`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p textora-markdown --lib augmenter::`

Expected: PASS，包括既有顶层段落硬换行测试。

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/augmenter.rs
git commit -m "$(cat <<'EOF'
fix: promote list and quote hard breaks on Enter

Reuse the shared hard-break boundary helper so a visual line break
inside a list item or quote becomes a structural marker break
instead of leaking backslashes or trailing spaces into the next line.
EOF
)"
```

---

### Task 2: 硬换行下一视觉行 Backspace 合并

**Files:**
- Modify: `crates/markdown/src/augmenter.rs`（`augment_backspace` 链、新增 `backspace_join_hard_break_line`）

**Interfaces:**
- Consumes: `hard_break_marker_ending_at`、`locate_source_line_bounds`、`newline_sequence_width_before`、`parse_list_marker`
- Produces: `fn backspace_join_hard_break_line(source: &str, current_byte: usize) -> Option<EditAugmentation>`，`replace_range = marker.start..current_byte`，`insert_text = ""`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn list_backspace_after_backslash_hard_break_joins_visual_lines() {
    let source = "- first\\\n  second";
    let current_byte = source.find("second").expect("fixture contains second");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace after a list hard break should join both visual lines");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- firstsecond");
    assert_eq!(augmentation.cursor_byte_after, "- first".len());
}

#[test]
fn list_backspace_after_double_space_hard_break_joins_visual_lines() {
    let source = "- first  \n  second";
    let current_byte = source.find("second").expect("fixture contains second");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace after a double-space list hard break should join both visual lines");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- firstsecond");
}

#[test]
fn quote_backspace_after_backslash_hard_break_joins_visual_lines() {
    let source = "> first\\\n> second";
    let current_byte = source.find("second").expect("fixture contains second");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("Backspace after a quote hard break should join both visual lines");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> firstsecond");
    assert_eq!(augmentation.cursor_byte_after, "> first".len());
}

#[test]
fn quote_backspace_after_crlf_hard_break_removes_the_complete_boundary() {
    let source = "> first\\\r\n> second";
    let current_byte = source.find("second").expect("fixture contains second");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
        .expect("CRLF quote hard break backspace should drop the whole boundary");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> firstsecond");
}

#[test]
fn backspace_does_not_join_hard_break_across_a_new_list_item() {
    let source = "- first\\\n- second";
    let current_byte = source.find("second").expect("fixture contains second");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace);
    let edited_source = augmentation
        .as_ref()
        .map(|aug| apply_augmentation_at(source, current_byte, aug))
        .unwrap_or_else(|| source.to_owned());

    assert_ne!(
        edited_source, "- firstsecond",
        "a following sibling item must not be glued onto the previous item"
    );
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-markdown --lib list_backspace_after_backslash_hard_break_joins_visual_lines quote_backspace_after_backslash_hard_break_joins_visual_lines`

Expected: FAIL。列表路径往往是 `None`（默认删一个缩进空格）；引用路径会先删 `> `，留下 `> first\\\nsecond`。

- [ ] **Step 3: 在 marker 删除之前合并硬换行行**

`augment_backspace` 在 `backspace_paragraph_boundary` 之前插入：

```rust
if let Some(aug) = backspace_join_hard_break_line(source, current_byte) {
    return Some(aug);
}
```

实现要点（完整函数写在 `previous_non_empty_line_end` 附近）：

```rust
fn backspace_join_hard_break_line(
    source: &str,
    current_byte: usize,
) -> Option<EditAugmentation> {
    let (line_start, _, _) = locate_source_line_bounds(source, current_byte)?;
    if current_byte < line_start {
        return None;
    }
    if !continuation_prefix_is_joinable(&source[line_start..current_byte]) {
        return None;
    }
    if line_starts_new_sibling_block(source, line_start) {
        return None;
    }
    let newline_width = newline_sequence_width_before(source, line_start)?;
    let previous_content_end = line_start - newline_width;
    let marker = hard_break_marker_ending_at(source, previous_content_end)?;
    let aug = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(marker.start..current_byte),
        cursor_byte_after: marker.start,
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}

fn continuation_prefix_is_joinable(prefix: &str) -> bool {
    prefix.bytes().all(|byte| matches!(byte, b' ' | b'\t' | b'>'))
}

fn line_starts_new_sibling_block(source: &str, line_start: usize) -> bool {
    let bytes = source.as_bytes();
    let mut probe = line_start;
    let mut leading_spaces = 0;
    while bytes.get(probe) == Some(&b' ') && leading_spaces < MAX_LEADING_BLOCK_INDENT {
        probe += 1;
        leading_spaces += 1;
    }
    parse_list_marker(source, probe).is_some()
        || heading_source_is_atx(source, line_start)
        || matches!(bytes.get(probe), Some(b'`' | b'~' | b'*' | b'-' | b'_'))
            && source[probe..].chars().take(3).all(|character| {
                matches!(character, '`' | '~' | '*' | '-' | '_')
            })
}
```

`line_starts_new_sibling_block` 对 HR/fence 的探测宁可偏严（少合并）不要把两项粘在一起。若第三条 `matches!` 误伤 `---` 续行，用「`parse_list_marker` 成功则一定不合并」作为必须条件，HR/fence 作为附加条件；以 Task 2 测试为准收紧。

- [ ] **Step 4: 跑 augmenter 测试**

Run: `cargo test -p textora-markdown --lib augmenter::`

Expected: PASS。既有 `backspace_after_backslash_hard_break_removes_the_marker`（顶层段落）仍通过。

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/augmenter.rs
git commit -m "$(cat <<'EOF'
fix: join list and quote visual lines across hard breaks

Backspace at the start of a hard-break continuation now deletes the
marker, newline, and line prefix together, before quote or list
marker stripping can run.
EOF
)"
```

---

### Task 3: 容器内已有换行上的 Enter 改为替换

**Files:**
- Modify: `crates/markdown/src/augmenter.rs`（`list_item_enter_augmentation`、`blockquote_enter_augmentation`）

**Interfaces:**
- Consumes: Task 1 的 `emit_marker_break_replacing`、`newline_sequence_width_at`
- Produces: 下一行是懒延续时，用 `\n{indent}{marker}` 替换该换行；下一行已有同类 marker 时走原来的 `emit_marker_break`（插入，不替换）。光标在换行之后的懒延续行首时只插入 `{indent}{marker}`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn list_enter_at_lazy_continuation_newline_becomes_a_new_item() {
    let source = "- item\npara";
    let current_byte = "- item".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on a lazy continuation newline should start a new item");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- item\n- para");
    assert_eq!(augmentation.cursor_byte_after, "- item\n- ".len());
}

#[test]
fn list_enter_after_lazy_continuation_newline_prefixes_the_following_line() {
    let source = "- item\npara";
    let current_byte = "- item\n".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter after a lazy continuation newline should mark the following line");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "- item\n- para");
    assert_eq!(augmentation.cursor_byte_after, "- item\n- ".len());
}

#[test]
fn quote_enter_at_lazy_continuation_newline_inserts_an_explicit_marker() {
    let source = "> first\nsecond";
    let current_byte = "> first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on a lazy quote newline should add an explicit marker");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> first\n> second");
    assert_eq!(augmentation.cursor_byte_after, "> first\n> ".len());
}

#[test]
fn quote_enter_between_explicit_lines_still_inserts_a_quote_line() {
    let source = "> first\n> second";
    let current_byte = "> first".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter between explicit quote lines should insert a quoted line");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "> first\n> \n> second");
    assert_eq!(augmentation.cursor_byte_after, "> first\n> ".len());
}
```

最后一条锁定「下一行已有 `>`」：必须插入 `> first\n> \n> second`，禁止替换成 `> first\n> > second`。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-markdown --lib list_enter_at_lazy_continuation_newline_becomes_a_new_item quote_enter_at_lazy_continuation_newline_inserts_an_explicit_marker`

Expected: FAIL，实际为 `"- item\n- \npara"` / `"> first\n> \nsecond"`（换行前又插了一个换行）。

- [ ] **Step 3: 最小实现**

在硬换行分支之后。先看换行后面那一行是不是已经带了结构前缀（列表：`parse_list_marker`；引用：去空白后是 `>`）。已有前缀则 `emit_marker_break`。否则：

```rust
if let Some(newline_width) = newline_sequence_width_at(source, current_byte) {
    let next_line_start = current_byte + newline_width;
    if !next_source_line_has_structure_prefix(source, next_line_start, indent, continuation_marker) {
        return Some(emit_marker_break_replacing(
            source,
            current_byte..next_line_start,
            indent,
            continuation_marker,
        ));
    }
}
if newline_sequence_width_before(source, current_byte).is_some()
    && !next_source_line_has_structure_prefix(source, current_byte, indent, continuation_marker)
{
    let insertion = format!("{indent}{continuation_marker}");
    let aug = EditAugmentation {
        insert_text: Some(insertion.clone()),
        replace_range: Some(current_byte..current_byte),
        cursor_byte_after: current_byte + insertion.len(),
    };
    debug_assert_augmentation(&aug, source);
    return Some(aug);
}
```

`next_source_line_has_structure_prefix`：从该字节跳过最多 `MAX_LEADING_BLOCK_INDENT` 个空格后，列表用 `parse_list_marker`，引用看是否以 `>` 开头。引用分支把 `indent`/`continuation_marker` 换成 `""` / `continuation_prefix`。非空项最后仍 `emit_marker_break`。

- [ ] **Step 4: 跑 augmenter 测试**

Run: `cargo test -p textora-markdown --lib augmenter::`

Expected: PASS。空列表项/空引用退出测试不得被这条替换逻辑误伤（`empty` 仍最先 return）。

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/augmenter.rs
git commit -m "$(cat <<'EOF'
fix: replace existing newlines when continuing lists and quotes

Enter on a lazy continuation boundary now rewrites that newline into
a marker break, so the following line becomes a real item or quoted
line instead of sitting behind an extra blank.
EOF
)"
```

---

### Task 4: 拆段时消耗紧邻的一个 ASCII 空格

**Files:**
- Modify: `crates/markdown/src/augmenter.rs`（`paragraph_enter_augmentation`、`heading_enter_augmentation`）

**Interfaces:**
- Consumes: `emit_block_break` / 标题中部单换行插入；`hard_break_boundary_after` 必须仍先于空格消耗
- Produces: `fn adjacent_split_space_range(source: &str, current_byte: usize) -> Option<Range<usize>>`，只匹配恰好一个 `b' '`，不匹配 tab，不匹配硬换行用的 ≥2 空格

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn paragraph_enter_before_a_single_space_does_not_leave_a_leading_space() {
    let source = "left right";
    let current_byte = "left".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("paragraph Enter should split at the word boundary");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "left\n\nright");
    assert_eq!(augmentation.cursor_byte_after, "left\n\n".len());
}

#[test]
fn paragraph_enter_after_a_single_space_does_not_leave_a_trailing_space() {
    let source = "left right";
    let current_byte = "left ".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("paragraph Enter should split at the word boundary");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "left\n\nright");
}

#[test]
fn heading_enter_before_a_single_space_does_not_leave_a_leading_space() {
    let source = "# left right";
    let current_byte = "# left".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("heading interior Enter should split at the word boundary");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "# left\nright");
}

#[test]
fn paragraph_enter_in_the_middle_of_a_word_keeps_letters_together() {
    let source = "left right";
    let current_byte = "le".len();

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("mid-word Enter should keep the split letters");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "le\n\nft right");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-markdown --lib paragraph_enter_before_a_single_space_does_not_leave_a_leading_space heading_enter_before_a_single_space_does_not_leave_a_leading_space`

Expected: FAIL，实际 `"left\n\n right"` / `"# left\n right"`。

- [ ] **Step 3: 最小实现**

```rust
fn adjacent_split_space_range(source: &str, current_byte: usize) -> Option<std::ops::Range<usize>> {
    let bytes = source.as_bytes();
    if current_byte > 0 && bytes.get(current_byte - 1) == Some(&b' ') {
        if current_byte >= 2 && bytes.get(current_byte - 2) == Some(&b' ') {
            return None;
        }
        if bytes.get(current_byte) == Some(&b' ') {
            return None;
        }
        return Some(current_byte - 1..current_byte);
    }
    if bytes.get(current_byte) == Some(&b' ') {
        if bytes.get(current_byte + 1) == Some(&b' ') {
            return None;
        }
        return Some(current_byte..current_byte + 1);
    }
    None
}
```

`paragraph_enter_augmentation` 在硬换行与 `cursor_touches_source_newline` 之后、`emit_block_break` 之前：若 `adjacent_split_space_range` 有值，则对该 range 做 `emit_block_break_replacing`（把那一个空格换成 `\n\n`）。

标题中部（`!at_end`）：若有该 range，则 `replace_range` 为该空格，`insert_text` 为单个 `preferred_newline_sequence`，光标在插入换行之后（即新段开头、`right` 之前）。

不要让这段逻辑抢在 `hard_break_boundary_after` 前面。

- [ ] **Step 4: 跑 augmenter 测试**

Run: `cargo test -p textora-markdown --lib augmenter::`

Expected: PASS。`enter_at_double_space_hard_break_promotes_it_to_a_block_boundary` 仍通过。

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/augmenter.rs
git commit -m "$(cat <<'EOF'
fix: drop the adjacent space when splitting a paragraph or heading

A word-boundary Enter no longer leaves a leading space on the new
block or a trailing space on the old one.
EOF
)"
```

---

### Task 5: 表格末行 Enter：新增行或退出

**Files:**
- Modify: `crates/markdown/src/augmenter.rs`（`EnterContext::TableCell`、`classify_enter_context` 的 table 命中、`enter_context_augmentation`）

**Interfaces:**
- Consumes: 已有 `table_cell_content_start`、`TableFrame.cell_ranges`、`preferred_newline_sequence`、`emit_block_break`
- Produces: 扩展后的

```rust
TableCell {
    next_cell_start: Option<usize>,
    column_count: usize,
    row_is_empty: bool,
    is_header_row: bool,
    row_line_end: usize,
}
```

非末行行为不变：`next_cell_start.is_some()` 时仍只移动光标。`next_cell_start.is_none()` 且 `row_is_empty && !is_header_row` 时删除当前行并在 `row_line_end` 处 `emit_block_break`。否则在 `row_line_end` 插入新行。

- [ ] **Step 1: 写失败测试**

先加一个分类探针，确认表头/表体行数（若 pulldown 把 separator 算进 range，用实测值锁 `row_line_end`）：

```rust
#[test]
fn table_enter_on_the_last_row_appends_a_new_row() {
    let source = "| a |\n|---|\n| b |";
    let current_byte = source.rfind('b').expect("fixture contains the body cell");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on the last table row should insert a new row");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "| a |\n|---|\n| b |\n|  |");
    assert_eq!(
        augmentation.cursor_byte_after,
        edited_source.rfind("|  |").expect("new row starts with an empty cell") + 2
    );
}

#[test]
fn table_enter_on_a_multi_column_last_row_copies_column_count() {
    let source = "| a | b |\n|---|---|\n| c | d |";
    let current_byte = source.rfind('c').expect("fixture contains the first body cell");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on the last table row should copy the column count");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "| a | b |\n|---|---|\n| c | d |\n|  |  |");
}

#[test]
fn table_enter_on_an_empty_last_body_row_exits_the_table() {
    let source = "| a |\n|---|\n|  |";
    let current_byte = source.rfind('|').expect("fixture contains the last pipe") - 1;

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on an empty last body row should leave the table");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "| a |\n|---|\n\n");
    assert_eq!(augmentation.cursor_byte_after, edited_source.len());
}

#[test]
fn table_enter_on_the_header_without_a_body_appends_a_body_row() {
    let source = "| a |\n|---|";
    let current_byte = source.find('a').expect("fixture contains the header cell");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter on a header with no body should create a body row");
    let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

    assert_eq!(edited_source, "| a |\n|---|\n|  |");
}

#[test]
fn table_enter_still_moves_to_the_cell_below_on_non_last_rows() {
    let source = "| a |\n|---|\n| b |";
    let current_byte = source.find('a').expect("fixture contains the header cell");

    let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
        .expect("Enter inside a table cell should move to the cell below");

    assert_eq!(augmentation.replace_range, None);
    assert_eq!(augmentation.insert_text.as_deref(), Some(""));
    assert_eq!(
        augmentation.cursor_byte_after,
        source.rfind('b').expect("fixture contains the cell below")
    );
}
```

最后一条是既有 `table_enter_moves_to_the_next_cell_content_start` 的行为锁定；若该测试已存在，不要重复，只跑旧测试确认未被破坏。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-markdown --lib table_enter_on_the_last_row_appends_a_new_row table_enter_on_an_empty_last_body_row_exits_the_table`

Expected: FAIL。当前末行 `next_cell_start = None`，光标不动、源码不变。

- [ ] **Step 3: 扩展分类并实现末行 Enter**

`TableFrame` 增加在 `Event::End(TagEnd::Table)` 里写入的 `table_end: usize`。命中单元格时计算：

```rust
let column_count = row.len();
let is_header_row = row_idx == 0;
let row_is_empty = row.iter().all(|cell| {
    source[cell.start..cell.end]
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t' | b'|' | b'\r' | b'\n'))
});
let last_cell_end = row.last().map(|cell| cell.end).unwrap_or(cell.end);
let row_line_end = source_line_content_end(
    source,
    locate_source_line_bounds(source, last_cell_end.saturating_sub(1.min(last_cell_end)))
        .map(|(_, _, end)| end)
        .unwrap_or(last_cell_end),
);
```

`enter_context_augmentation` 中 `TableCell` 分支：

```rust
EnterContext::TableCell {
    next_cell_start,
    column_count,
    row_is_empty,
    is_header_row,
    row_line_end,
} => {
    if let Some(next_cell_start) = next_cell_start {
        Some(EditAugmentation {
            insert_text: Some(String::new()),
            replace_range: None,
            cursor_byte_after: next_cell_start,
        })
    } else if row_is_empty && !is_header_row {
        table_exit_empty_row_augmentation(source, row_line_end)
    } else {
        Some(table_insert_row_augmentation(source, row_line_end, column_count))
    }
}
```

新行文本：`column_count` 个 `|  ` 再加闭合 `|`，前面加 `preferred_newline_sequence`。插入点：若 `row_line_end < source.len()` 且该处是换行，插在换行**之后**（避免 `| b |\n\n|  |`）；若已在 EOF，直接追加。光标：插入串里第一格内容空格的位置（`|{nl}| ` 之后那个内容空格，即 `row_line_end + newline.len() + 2`，对标准 `|  |` 行）。

空表体行退出：`replace_range` 为该行（含其前导换行，使 `|---|\n|  |` 变成 `|---|`），再在删除后的表尾 `emit_block_break`。若一次 replacement 难以表达，允许 `replace_range` 覆盖 `newline_before_row..row_line_end` 且 `insert_text` 为两个换行序列。最终源码必须是 `"| a |\n|---|\n\n"`，光标在文档末尾。

同步修改所有 `matches!(EnterContext::TableCell { .. })` 与构造处，否则无法编译。

- [ ] **Step 4: 跑 augmenter 测试**

Run: `cargo test -p textora-markdown --lib augmenter::`

Expected: PASS，包括旧的 `table_enter_moves_to_the_next_cell_content_start` 与 `table_enter_into_an_empty_cell_stops_at_the_cell_end`。

- [ ] **Step 5: Commit**

```bash
git add crates/markdown/src/augmenter.rs
git commit -m "$(cat <<'EOF'
fix: insert or exit on Enter in the last table row

A populated last row grows a new source row with the same column
count. An empty body row leaves the table the same way an empty
list item does.
EOF
)"
```

---

### Task 6: 把行为写进 2026-08-02 规范

**Files:**
- Modify: `docs/specs/2026-08-02-markdown-wysiwyg-enter-backspace-behavior.md`

**Interfaces:**
- Consumes: 本计划 Task 1–5 的最终源码断言
- Produces: 规范矩阵新增列表/引用/硬换行/表格/空格行；「当前范围」不再把列表、引用、表格写成「既有专用策略」而不写结果

- [ ] **Step 1: 扩展 Enter 矩阵（直接写入规范，不改代码）**

在既有 Enter 表后追加（`|` 仍表示光标）：

| 场景 | Enter 前 | Enter 后 | 约束 |
|---|---|---|---|
| 段内单空格前 | `left\| right` | `left\n\n\|right` | 吃掉那一个空格 |
| 段内单空格后 | `left \|\right` | `left\n\n\|right` | 同上 |
| 列表硬换行边界 | `- first\\|\n  second` | `- first\n- \|second` | 不残留 `\` |
| 列表懒延续换行 | `- item\|\npara` | `- item\n- \|para` | 替换该 `\n`，不插第三个 |
| 引用硬换行边界 | `> first\\|\n> second` | `> first\n> \|second` | 不残留 `\` |
| 引用懒延续换行 | `> first\|\nsecond` | `> first\n> \|second` | 后行补显式 `>` |
| 表格非末行 | 格内 | 源码不变，光标到下一行同列 | 已有 |
| 表格末行有内容 | `\| b \|` 内 | 表末多一行 `\|  \|` | 列数与当前行相同 |
| 表格空表体末行 | 空格内 | 删除该行，表后 `\n\n\|` | 不删表头 |

Backspace 表追加：

| 场景 | Backspace 前 | Backspace 后 | 约束 |
|---|---|---|---|
| 列表硬换行下行首 | `- first\\\n  \|second` | `- first\|second` | 先于 marker 删除 |
| 引用硬换行下行首 | `> first\\\n> \|second` | `> first\|second` | 先于去掉 `>` |
| 硬换行跨新列表项 | `- first\\\n- \|second` | 不合并两项 | 交给既有 marker/默认路径 |

把文末「当前范围」改成：段落、ATX、Setext 无选区、列表项、引用行、表格单元格、块间空段；Shift+Enter、HTML `<br>`、项内再开段落仍不在范围。

- [ ] **Step 2: 对照测试名扫一遍，规范每一行都能指到 Task 1–5 的测试**

若某行没有测试，回到对应 Task 补测，不要在规范里写未实现行为。

- [ ] **Step 3: Commit**

```bash
git add docs/specs/2026-08-02-markdown-wysiwyg-enter-backspace-behavior.md
git commit -m "$(cat <<'EOF'
docs: record list, quote, and table linebreak Enter rules

Extend the 2026-08-02 WYSIWYG spec so container hard breaks, lazy
continuation Enter, split-space trimming, and last-row table Enter
match the augmenter tests.
EOF
)"
```

---

## 验证（全部 Task 完成后）

```bash
cargo fmt --all
cargo test -p textora-markdown --lib
cargo clippy -p textora-markdown --all-targets -- -D warnings
```

最终验收：

- 顶层段落硬换行 8 月 19 日测试全部仍绿
- 列表/引用：反斜杠、三反斜杠、双空格、CRLF 的 Enter 与 Backspace
- 列表/引用懒延续 Enter 不产生多余空行
- 段落/标题单空格拆分不留空格；中词拆分保留字母
- 表格：非末行跳格、末行新增、多列、空表体退出、无表体时表头 Enter 增行
- 不实现 Shift+Enter，不改 HTML 渲染

## Spec coverage

| 设计要求 | Task |
|---|---|
| 列表/引用硬换行 Enter 升级为结构分隔 | 1 |
| 奇偶反斜杠、双空格、CRLF | 1、2 |
| 硬换行下行 Backspace 合并，先于 marker 删除 | 2 |
| 不跨新列表项合并 | 2 |
| 懒延续换行上 Enter 替换而非预插 | 3 |
| 拆段吃掉一个 ASCII 空格 | 4 |
| 表格末行新增 / 空行退出 | 5 |
| 规范矩阵 | 6 |
| Shift+Enter、HTML、SourceLineMap | 明确非目标，无 Task |
