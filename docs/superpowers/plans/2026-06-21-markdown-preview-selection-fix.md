# Markdown Preview 选区修复 — 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 BlockQuote/Table 内无法选文本、点击空白区域清除选区两个 bug，通过将选区坐标从分层 (block_idx, line_idx) 重构为扁平行索引。

**Architecture:** 在 LazyLayout 中新增 `flat_lines: Vec<FlatLine>`，按阅读序递归展平所有块→行。PreviewPos 从 3 元组改为 `(flat_line_idx, char_pos)`。所有选区操作（hit test、渲染、文本提取、键盘导航）统一索引 `flat_lines`，消除 `block_lines()` 返回 `None` 的所有分支。

**Tech Stack:** Rust, 无新增依赖

## Global Constraints

- 不改动 shaping 模块、markdown parser、文本布局核心
- 不改变 LaidOutDoc / LaidOutBlock 的数据结构
- 所有 pub(crate) 接口变更在 `crates/app` 和 `crates/markdown` 内完成
- 编译必须通过 `cargo check --workspace`

---

### Task 1: 在 LazyLayout 中新增 FlatLine 和 build_flat_lines

**Files:**
- Modify: `crates/markdown/src/layout.rs` — 新增 FlatLine 结构体、扩展 LazyLayout、实现 build_flat_lines

**Interfaces:**
- Produces: `FlatLine` struct, `LazyLayout.flat_lines: Vec<FlatLine>`, `LazyLayout::build_flat_lines()`

- [ ] **Step 1: 在 LaidOutBlockKind 定义之后、impl LazyLayout 之前添加 FlatLine 结构体**

在 `crates/markdown/src/layout.rs` 中，`LaidOutBlockKind` enum 定义之后、`impl LazyLayout` 之前（约第 328 行后），添加：

```rust
/// A text line with absolute document position, for flat-indexed selection.
#[derive(Clone, Debug)]
pub struct FlatLine {
    /// 0-based index in the flat reading-order array.
    pub flat_idx: usize,
    /// Line rect with y in absolute document coordinates (block y + delta + relative line y).
    pub rect: Rect,
    pub text: String,
    pub font_size: f32,
    pub shaped: Option<shaping::ShapedRun>,
}
```

**注意：** `crates/shaping/src/lib.rs` 的 `ShapedRun` 已 derive `Clone`，且 `crates/markdown/src/layout.rs` 已有 `use shaping::...`。

- [ ] **Step 2: 在 LazyLayout 中新增 flat_lines 字段**

修改 `LazyLayout` struct（约第 29 行）：

```rust
// 在 y_delta 字段之后、laid_to_doc 字段之前添加：
/// Flattened lines in reading order, for selection indexing.
pub flat_lines: Vec<FlatLine>,
```

完整 struct 变为：
```rust
#[derive(Clone, Debug)]
pub struct LazyLayout {
    pub doc: MarkdownDoc,
    pub laid_out: LaidOutDoc,
    pub precise: Vec<bool>,
    pub y_delta: Vec<f32>,
    pub flat_lines: Vec<FlatLine>,   // ← 新增
    laid_to_doc: Vec<usize>,
}
```

- [ ] **Step 3: 在 LaidOutDoc 中新增 build_flat_lines 方法**

在 `impl LazyLayout` 块内（`laid_to_doc` 方法之后），添加：

```rust
/// Build the flat line array from the laid-out blocks.
/// Must be called after layout/precision changes.
pub fn build_flat_lines(&mut self) {
    self.flat_lines.clear();
    let mut flat_idx = 0usize;
    for (bi, block) in self.laid_out.blocks.iter().enumerate() {
        let y_delta = self.y_delta.get(bi).copied().unwrap_or(0.0);
        self.flatten_block(bi, block, block.rect.y + y_delta, &mut flat_idx);
    }
}

fn flatten_block(
    &mut self,
    block_idx: usize,
    block: &LaidOutBlock,
    abs_block_y: f32,
    flat_idx: &mut usize,
) {
    match &block.kind {
        LaidOutBlockKind::Text { lines }
        | LaidOutBlockKind::CodeBlock { lines, .. }
        | LaidOutBlockKind::MetadataBlock { lines } => {
            for line in lines {
                self.flat_lines.push(FlatLine {
                    flat_idx: *flat_idx,
                    rect: Rect::new(
                        line.rect.x,
                        abs_block_y + line.rect.y,
                        line.rect.w,
                        line.rect.h,
                    ),
                    text: line.text.clone(),
                    font_size: line.font_size,
                    shaped: line.shaped.clone(),
                });
                *flat_idx += 1;
            }
        }
        LaidOutBlockKind::BlockQuote { blocks: sub_blocks } => {
            for sub in sub_blocks {
                self.flatten_block(block_idx, sub, abs_block_y + sub.rect.y, flat_idx);
            }
        }
        LaidOutBlockKind::ListItem { lines, blocks: sub_blocks, .. } => {
            for line in lines {
                self.flat_lines.push(FlatLine {
                    flat_idx: *flat_idx,
                    rect: Rect::new(
                        line.rect.x,
                        abs_block_y + line.rect.y,
                        line.rect.w,
                        line.rect.h,
                    ),
                    text: line.text.clone(),
                    font_size: line.font_size,
                    shaped: line.shaped.clone(),
                });
                *flat_idx += 1;
            }
            for sub in sub_blocks {
                self.flatten_block(block_idx, sub, abs_block_y + sub.rect.y, flat_idx);
            }
        }
        LaidOutBlockKind::Table { header, rows, .. } => {
            // Header: columns, each with lines
            for cell_lines in header {
                for line in cell_lines {
                    self.flat_lines.push(FlatLine {
                        flat_idx: *flat_idx,
                        rect: Rect::new(
                            line.rect.x,
                            abs_block_y + line.rect.y,
                            line.rect.w,
                            line.rect.h,
                        ),
                        text: line.text.clone(),
                        font_size: line.font_size,
                        shaped: line.shaped.clone(),
                    });
                    *flat_idx += 1;
                }
            }
            // Body: rows -> columns -> cell lines
            for row in rows {
                for cell_lines in row {
                    for line in cell_lines {
                        self.flat_lines.push(FlatLine {
                            flat_idx: *flat_idx,
                            rect: Rect::new(
                                line.rect.x,
                                abs_block_y + line.rect.y,
                                line.rect.w,
                                line.rect.h,
                            ),
                            text: line.text.clone(),
                            font_size: line.font_size,
                            shaped: line.shaped.clone(),
                        });
                        *flat_idx += 1;
                    }
                }
            }
        }
        LaidOutBlockKind::HorizontalRule => {
            // Synthetic line — empty text, minimal height
            self.flat_lines.push(FlatLine {
                flat_idx: *flat_idx,
                rect: Rect::new(block.rect.x, abs_block_y, block.rect.w, 1.0),
                text: String::new(),
                font_size: 14.0,
                shaped: None,
            });
            *flat_idx += 1;
        }
    }
}
```

- [ ] **Step 4: 在 LazyLayout 构造和 y_delta 更新点调用 build_flat_lines**

有两个调用点，必须都加：

**A. `from_doc` 末尾（line 120 附近）**

修改 `Self { doc, laid_out, precise: vec![false; n], y_delta: vec![0.0f32; n], laid_to_doc }` 为：

```rust
let mut this = Self {
    doc,
    laid_out,
    precise: vec![false; n],
    y_delta: vec![0.0f32; n],
    laid_to_doc,
    flat_lines: Vec::new(),
};
this.build_flat_lines();
this
```

**B. `ensure_precise_range` 末尾（line 182-183），`apply_deltas` 之后**

在 `apply_deltas(&mut self.y_delta, &deltas);` 之后、`deltas` 之前加入：

```rust
if !deltas.is_empty() {
    apply_deltas(&mut self.y_delta, &deltas);
    self.build_flat_lines();  // ← y_delta 变更后重建扁平行数组
}
```

这样 flat_lines 在初始构造和每次 precision pass 后都保持与 y_delta 同步。

- [ ] **Step 5: 编译验证**

```bash
cargo check -p edit_plus_markdown
```

预期：通过（仅新增未使用的 struct，可能有 warning，后续任务会消除）

- [ ] **Step 6: Commit**

```bash
git add crates/markdown/src/layout.rs
git commit -m "feat(markdown): add FlatLine and build_flat_lines to LazyLayout"
```

---

### Task 2: 重构 PreviewPos 并删除 block_lines/block_count

**Files:**
- Modify: `crates/app/src/md_preview.rs:21-25` — PreviewPos 改2字段
- Modify: `crates/app/src/md_preview.rs:916-934` — 删除 block_count 和 block_lines
- Modify: `crates/app/src/md_preview.rs:624-637` — preview_selection_range 简化

**Interfaces:**
- Consumes: `LazyLayout.flat_lines: Vec<FlatLine>` (from Task 1)
- Produces: `PreviewPos { flat_line_idx, char_pos }`

- [ ] **Step 1: 修改 PreviewPos 结构体**

将 `crates/app/src/md_preview.rs:21-25` 改为：

```rust
/// Position within the preview's rendered text.
/// flat_line_idx indexes into LazyLayout.flat_lines (reading-order).
/// char_pos is the character offset within the line's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreviewPos {
    pub flat_line_idx: usize,
    pub char_pos: usize,
}
```

- [ ] **Step 2: 删除 block_count 和 block_lines 方法**

删除 `crates/app/src/md_preview.rs:916-934` 中的 `block_count()` 和 `block_lines()` 两个方法。

- [ ] **Step 3: 简化 preview_selection_range**

将 `crates/app/src/md_preview.rs:624-637` 改为：

```rust
pub(crate) fn preview_selection_range(&self) -> Option<(PreviewPos, PreviewPos)> {
    let anchor = self.sel_anchor?;
    let cursor = self.sel_cursor?;
    if anchor == cursor {
        return None;
    }
    if (anchor.flat_line_idx, anchor.char_pos)
        <= (cursor.flat_line_idx, cursor.char_pos)
    {
        Some((anchor, cursor))
    } else {
        Some((cursor, anchor))
    }
}
```

- [ ] **Step 4: 编译验证（预期大量编译错误，确认都是因为引用旧字段）**

```bash
cargo check -p edit_plus_app 2>&1 | head -80
```

预期：大量错误，都源于 `pos.block_idx`、`pos.line_idx`、`block_lines()`、`block_count()` 的引用。这些在后续任务中逐一修复。

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/md_preview.rs
git commit -m "refactor(preview): change PreviewPos to flat_line_idx, remove block_lines"
```

---

### Task 3: 重写 hit test 为扁平行扫描 + 吸附逻辑

**Files:**
- Modify: `crates/app/src/md_preview.rs:467-553` — 重写 preview_hit_test，删除 hit_test_blocks
- Modify: `crates/app/src/md_preview.rs:557-601` — char_at_x / char_x 适配 FlatLine

**Interfaces:**
- Consumes: `FlatLine.rect`, `FlatLine.text`, `FlatLine.font_size`, `FlatLine.shaped`
- Produces: `preview_hit_test()` 返回 `Option<PreviewPos>`（空文档时 None，否则 snap 总返回 Some）

- [ ] **Step 1: 删除 hit_test_blocks 并重写 preview_hit_test**

替换 `crates/app/src/md_preview.rs` 中 `preview_hit_test`（约第 467 行）到 `hit_test_blocks` 结束（约第 553 行）为：

```rust
/// Hit-test a screen pixel position to a PreviewPos in the rendered text.
/// Snaps to the nearest line for clicks in empty space.
pub(crate) fn preview_hit_test(
    &self,
    px: f32,
    py: f32,
    offset_x: f32,
    offset_y: f32,
) -> Option<PreviewPos> {
    let lazy = self.lazy.as_ref()?;
    let flat_lines = &lazy.flat_lines;
    if flat_lines.is_empty() {
        return None;
    }
    let doc_x = px - offset_x;
    let doc_y = py - offset_y + self.scroll_y;

    // Binary search for the line containing doc_y, or the closest line
    let idx = match flat_lines.binary_search_by(|fl| {
        if doc_y < fl.rect.y {
            std::cmp::Ordering::Greater
        } else if doc_y > fl.rect.y + fl.rect.h {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(i) => i,
        Err(i) => {
            // doc_y is between lines (or above/below all lines).
            // Find the closest line by vertical center distance.
            if i == 0 {
                0 // above first line
            } else if i >= flat_lines.len() {
                flat_lines.len() - 1 // below last line
            } else {
                // Between two lines: pick the one whose center is closer
                let dist_prev = doc_y - (flat_lines[i - 1].rect.y + flat_lines[i - 1].rect.h * 0.5);
                let dist_next = (flat_lines[i].rect.y + flat_lines[i].rect.h * 0.5) - doc_y;
                if dist_prev.abs() <= dist_next.abs() { i - 1 } else { i }
            }
        }
    };

    let line = &flat_lines[idx];
    let rel_x = doc_x - line.rect.x;
    let char_pos = if rel_x <= 0.0 { 0 } else { self.char_at_x(line, rel_x) };
    Some(PreviewPos { flat_line_idx: idx, char_pos })
}
```

- [ ] **Step 2: 修改 char_at_x 接受 &FlatLine**

将 `crates/app/src/md_preview.rs:557-589` 的函数签名改为：

```rust
fn char_at_x(&self, flat_line: &edit_plus_markdown::layout::FlatLine, rel_x: f32) -> usize {
```

然后将其内部所有 `line.text` 改为 `flat_line.text`，所有 `line.font_size` 改为 `flat_line.font_size`，所有 `line.shaped` 改为 `flat_line.shaped`。

- [ ] **Step 3: 修改 char_x 接受 &FlatLine**

同样改 `fn char_x`（约第 592 行）：

```rust
fn char_x(&self, flat_line: &edit_plus_markdown::layout::FlatLine, char_pos: usize) -> f32 {
```

内部 `line.text` → `flat_line.text`，`line.shaped` → `flat_line.shaped`。

- [ ] **Step 4: 编译验证**

```bash
cargo check -p edit_plus_app 2>&1 | head -60
```

预期：hit test 相关错误清零，还有 selection_highlights / preview_selected_text / word_at_pos 等引用旧字段的错误。

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/md_preview.rs
git commit -m "refactor(preview): rewrite hit-test with flat-line scan and snap-to-nearest"
```

---

### Task 4: 更新 word_at_pos / line_range_at_pos

**Files:**
- Modify: `crates/app/src/md_preview.rs:679-785`

- [ ] **Step 1: 重写 word_at_pos**

```rust
pub(crate) fn word_at_pos(&self, pos: PreviewPos) -> (PreviewPos, PreviewPos) {
    let Some(ref lazy) = self.lazy else {
        return (pos, pos);
    };
    let flat_lines = &lazy.flat_lines;
    let Some(line) = flat_lines.get(pos.flat_line_idx) else {
        return (pos, pos);
    };
    let text = &line.text;
    if text.is_empty() {
        return (pos, pos);
    }

    let char_count = text.chars().count();
    let pos_char = pos.char_pos.min(char_count);

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Class { Word, Whitespace, Punctuation }
    fn char_class(ch: char) -> Class {
        if ch.is_alphanumeric() || ch == '_' { Class::Word }
        else if ch.is_whitespace() { Class::Whitespace }
        else { Class::Punctuation }
    }

    if pos_char >= char_count {
        // (same end-of-text logic as before, using flat_line_idx)
        if char_count == 0 { return (pos, pos); }
        let last_ch = text.chars().last().unwrap();
        let last_class = char_class(last_ch);
        if last_class == Class::Whitespace { return (pos, pos); }
        let mut start = char_count - 1;
        for (i, ch) in text.chars().rev().enumerate() {
            if char_class(ch) != last_class { start = char_count - i; break; }
            if char_count - 1 - i == 0 { start = 0; break; }
        }
        return (
            PreviewPos { flat_line_idx: pos.flat_line_idx, char_pos: start },
            PreviewPos { flat_line_idx: pos.flat_line_idx, char_pos: char_count },
        );
    }

    let chars: Vec<char> = text.chars().collect();
    let clicked_class = char_class(chars[pos_char]);

    if clicked_class == Class::Whitespace {
        let mut s = pos_char;
        while s > 0 && char_class(chars[s - 1]) == Class::Whitespace { s -= 1; }
        let mut e = pos_char;
        while e < chars.len() && char_class(chars[e]) == Class::Whitespace { e += 1; }
        return (
            PreviewPos { flat_line_idx: pos.flat_line_idx, char_pos: s },
            PreviewPos { flat_line_idx: pos.flat_line_idx, char_pos: e },
        );
    }

    let mut start = pos_char;
    while start > 0 && char_class(chars[start - 1]) == clicked_class { start -= 1; }
    let mut end = pos_char;
    while end < chars.len() && char_class(chars[end]) == clicked_class { end += 1; }

    (
        PreviewPos { flat_line_idx: pos.flat_line_idx, char_pos: start },
        PreviewPos { flat_line_idx: pos.flat_line_idx, char_pos: end },
    )
}
```

- [ ] **Step 2: 重写 line_range_at_pos**

```rust
pub(crate) fn line_range_at_pos(&self, pos: PreviewPos) -> (PreviewPos, PreviewPos) {
    let char_count = match self.lazy.as_ref() {
        Some(lazy) => lazy.flat_lines
            .get(pos.flat_line_idx)
            .map_or(0, |fl| fl.text.chars().count()),
        None => 0,
    };
    (
        PreviewPos { flat_line_idx: pos.flat_line_idx, char_pos: 0 },
        PreviewPos { flat_line_idx: pos.flat_line_idx, char_pos: char_count },
    )
}
```

- [ ] **Step 3: 编译验证**

```bash
cargo check -p edit_plus_app 2>&1 | head -40
```

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/md_preview.rs
git commit -m "refactor(preview): update word_at_pos and line_range_at_pos for flat lines"
```

---

### Task 5: 重写 selection_highlights 和 preview_selected_text

**Files:**
- Modify: `crates/app/src/md_preview.rs:790-908`

- [ ] **Step 1: 重写 preview_selected_text**

```rust
pub(crate) fn preview_selected_text(&self) -> Option<String> {
    let (start, end) = self.preview_selection_range()?;
    let Some(ref lazy) = self.lazy else {
        return None;
    };
    let flat_lines = &lazy.flat_lines;
    let end_idx = end.flat_line_idx.min(flat_lines.len().saturating_sub(1));

    let mut result = String::new();
    for idx in start.flat_line_idx..=end_idx {
        let line = &flat_lines[idx];
        let text = &line.text;
        let ch_start = if idx == start.flat_line_idx { start.char_pos } else { 0 };
        let ch_end = if idx == end.flat_line_idx {
            end.char_pos
        } else {
            text.chars().count()
        };
        let byte_start = text.char_indices().nth(ch_start).map(|(i, _)| i).unwrap_or(text.len());
        let byte_end = text.char_indices().nth(ch_end).map(|(i, _)| i).unwrap_or(text.len());
        if byte_start < byte_end {
            result.push_str(&text[byte_start..byte_end]);
        }
        let is_last = idx == end_idx;
        if !is_last {
            result.push('\n');
        }
    }
    Some(result)
}
```

- [ ] **Step 2: 重写 selection_highlights**

```rust
pub(crate) fn selection_highlights(&self, sel_color: [f32; 4]) -> DrawList {
    let mut dl = DrawList::new();
    let (start, end) = match self.preview_selection_range() {
        Some(r) => r,
        None => return dl,
    };
    let Some(ref lazy) = self.lazy else {
        return dl;
    };
    let flat_lines = &lazy.flat_lines;
    let ox = self.cached_offset_x;
    let oy = self.cached_offset_y;
    let viewport_h = self.cached_dl_viewport.1;
    let end_idx = end.flat_line_idx.min(flat_lines.len().saturating_sub(1));

    for idx in start.flat_line_idx..=end_idx {
        let line = &flat_lines[idx];
        let line_y = line.rect.y - self.scroll_y + oy;
        let line_h = line.rect.h;
        if line_y + line_h < 0.0 || line_y > viewport_h {
            continue;
        }

        let ch_start = if idx == start.flat_line_idx { start.char_pos } else { 0 };
        let ch_end = if idx == end.flat_line_idx {
            end.char_pos
        } else {
            line.text.chars().count()
        };

        let x0 = line.rect.x + ox + self.char_x(line, ch_start);
        let is_end_line = idx == end.flat_line_idx;
        let x1 = if !is_end_line && ch_end >= line.text.chars().count() {
            line.rect.x + ox + line.rect.w
        } else {
            line.rect.x + ox + self.char_x(line, ch_end)
        };
        let w = (x1 - x0).max(0.0);
        if w > 0.0 {
            dl.cmds.push(DrawCmd::FillRect {
                rect: Rect::new(x0, line_y, w, line_h),
                color: sel_color,
                radius: 0.0,
            });
        }
    }
    dl
}
```

- [ ] **Step 3: 编译验证**

```bash
cargo check -p edit_plus_app 2>&1 | head -40
```

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/md_preview.rs
git commit -m "refactor(preview): rewrite selection highlights and text extraction for flat lines"
```

---

### Task 6: 更新 preview_select_all

**Files:**
- Modify: `crates/app/src/md_preview.rs:646-672`

- [ ] **Step 1: 重写 preview_select_all**

```rust
pub fn preview_select_all(&mut self) {
    let Some(ref lazy) = self.lazy else {
        return;
    };
    let flat_lines = &lazy.flat_lines;
    if flat_lines.is_empty() {
        return;
    }
    self.sel_anchor = Some(PreviewPos { flat_line_idx: 0, char_pos: 0 });
    let last_idx = flat_lines.len() - 1;
    let last_char = flat_lines[last_idx].text.chars().count();
    self.sel_cursor = Some(PreviewPos { flat_line_idx: last_idx, char_pos: last_char });
}
```

- [ ] **Step 2: 编译验证**

```bash
cargo check -p edit_plus_app 2>&1 | head -20
```

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/md_preview.rs
git commit -m "refactor(preview): update preview_select_all for flat lines"
```

---

### Task 7: 更新键盘导航命令（dispatch/editor.rs）

**Files:**
- Modify: `crates/app/src/dispatch/editor.rs:78-200`

- [ ] **Step 1: 重写 ExtendLeft**

```rust
EditCommand::ExtendLeft => {
    if let Some(crate::view::View::Markdown(mv)) = self.workspace.active_view_mut() {
        if mv.preview.sel_anchor.is_none() {
            mv.preview.sel_anchor = mv.preview.sel_cursor;
        }
        if let Some(cursor) = mv.preview.sel_cursor {
            let mut new_cursor = cursor;
            if cursor.char_pos > 0 {
                new_cursor.char_pos -= 1;
            } else if cursor.flat_line_idx > 0 {
                new_cursor.flat_line_idx -= 1;
                new_cursor.char_pos = mv.preview.lazy.as_ref()
                    .and_then(|l| l.flat_lines.get(new_cursor.flat_line_idx))
                    .map_or(0, |fl| fl.text.chars().count());
            }
            mv.preview.sel_cursor = Some(new_cursor);
        }
        effect = effect.merge(AppEffect::REDRAW);
    }
    return effect;
}
```

- [ ] **Step 2: 重写 ExtendRight**

```rust
EditCommand::ExtendRight => {
    if let Some(crate::view::View::Markdown(mv)) = self.workspace.active_view_mut() {
        if mv.preview.sel_anchor.is_none() {
            mv.preview.sel_anchor = mv.preview.sel_cursor;
        }
        if let Some(cursor) = mv.preview.sel_cursor {
            let mut new_cursor = cursor;
            if let Some(lazy) = mv.preview.lazy.as_ref() {
                if let Some(line) = lazy.flat_lines.get(cursor.flat_line_idx) {
                    let line_len = line.text.chars().count();
                    if cursor.char_pos < line_len {
                        new_cursor.char_pos += 1;
                    } else if cursor.flat_line_idx + 1 < lazy.flat_lines.len() {
                        new_cursor.flat_line_idx += 1;
                        new_cursor.char_pos = 0;
                    }
                }
            }
            mv.preview.sel_cursor = Some(new_cursor);
        }
        effect = effect.merge(AppEffect::REDRAW);
    }
    return effect;
}
```

- [ ] **Step 3: 重写 ExtendToLineEnd**

```rust
EditCommand::ExtendToLineEnd => {
    if let Some(crate::view::View::Markdown(mv)) = self.workspace.active_view_mut() {
        if mv.preview.sel_anchor.is_none() {
            mv.preview.sel_anchor = mv.preview.sel_cursor;
        }
        if let Some(cursor) = mv.preview.sel_cursor {
            let char_count = mv.preview.lazy.as_ref()
                .and_then(|l| l.flat_lines.get(cursor.flat_line_idx))
                .map_or(0, |fl| fl.text.chars().count());
            mv.preview.sel_cursor = Some(PreviewPos {
                flat_line_idx: cursor.flat_line_idx,
                char_pos: char_count,
            });
        }
        effect = effect.merge(AppEffect::REDRAW);
    }
    return effect;
}
```

- [ ] **Step 4: 重写 ExtendToDocStart（简单——(0, 0)）**

已经在 Task 2 中隐含正确（`(0, 0)` 现在就是 `(flat_line_idx: 0, char_pos: 0)`），确认无误即可。

- [ ] **Step 5: 重写 ExtendToDocEnd**

```rust
EditCommand::ExtendToDocEnd => {
    if let Some(crate::view::View::Markdown(mv)) = self.workspace.active_view_mut() {
        if mv.preview.sel_anchor.is_none() {
            mv.preview.sel_anchor = mv.preview.sel_cursor;
        }
        if let Some(lazy) = mv.preview.lazy.as_ref() {
            let last_idx = lazy.flat_lines.len().saturating_sub(1);
            let last_char = lazy.flat_lines.last()
                .map_or(0, |fl| fl.text.chars().count());
            mv.preview.sel_cursor = Some(PreviewPos {
                flat_line_idx: last_idx,
                char_pos: last_char,
            });
        }
        effect = effect.merge(AppEffect::REDRAW);
    }
    return effect;
}
```

- [ ] **Step 6: 确认 ExtendToLineStart 无需改动**

`ExtendToLineStart` 只设置 `char_pos = 0`，不涉及 `flat_line_idx`。确认代码已正确。

- [ ] **Step 7: 编译验证**

```bash
cargo check -p edit_plus_app 2>&1 | head -40
```

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/dispatch/editor.rs
git commit -m "refactor(preview): update keyboard navigation for flat line indices"
```

---

### Task 8: 更新鼠标事件分发（dispatch/mouse.rs）

**Files:**
- Modify: `crates/app/src/dispatch/mouse.rs:88-117` — 拖拽时三元组比较改为二元组

- [ ] **Step 1: 更新 dispatch_editor_cursor_moved 中的拖拽比较**

将第 93-112 行的两处三元组比较（`pos.block_idx, pos.line_idx, pos.char_pos`）改为二元组：

```rust
if self.mouse.click_count >= 3 {
    let (line_start, line_end) = mv.preview.line_range_at_pos(pos);
    let anchor = mv.preview.sel_anchor.unwrap_or(pos);
    if (pos.flat_line_idx, pos.char_pos)
        >= (anchor.flat_line_idx, anchor.char_pos)
    {
        mv.preview.sel_cursor = Some(line_end);
    } else {
        mv.preview.sel_cursor = Some(line_start);
    }
} else if self.mouse.click_count >= 2 {
    let (word_start, word_end) = mv.preview.word_at_pos(pos);
    let anchor = mv.preview.sel_anchor.unwrap_or(pos);
    if (pos.flat_line_idx, pos.char_pos)
        >= (anchor.flat_line_idx, anchor.char_pos)
    {
        mv.preview.sel_cursor = Some(word_end);
    } else {
        mv.preview.sel_cursor = Some(word_start);
    }
} else {
    mv.preview.sel_cursor = Some(pos);
}
```

- [ ] **Step 2: 确认 dispatch_editor_mouse_input 中的 clear 逻辑正确**

第 50-51 行——`preview_pos` 为 None 时调用 `clear_preview_selection()`。由于 hit test 现在只在空文档时返回 None，其他情况 snap 总有结果，这个逻辑是正确的。

- [ ] **Step 3: 编译验证**

```bash
cargo check -p edit_plus_app 2>&1
```

预期：编译通过（无错误）。

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/dispatch/mouse.rs
git commit -m "refactor(preview): update mouse drag comparison for flat line indices"
```

---

### Task 9: 全工作区编译与清理

**Files:**
- Modify: `crates/app/src/md_preview.rs` — 清理未使用 import/死代码警告

- [ ] **Step 1: 全工作区编译检查**

```bash
cargo check --workspace 2>&1
```

预期：通过。如有 warning 关于未使用的 `LaidOutBlockKind` variant 匹配（已全部覆盖），确认无新增 warning。

- [ ] **Step 2: Clippy 检查**

```bash
cargo clippy --workspace 2>&1 | tail -20
```

目标：无新增 clippy 警告（如有，修复）。

- [ ] **Step 3: 清理死代码**

检查是否有因删除 `block_lines`/`block_count`/`hit_test_blocks` 产生的未使用 import。特别检查 `md_preview.rs` 顶部的 `use` 语句。

- [ ] **Step 4: 运行现有测试**

```bash
cargo test -p edit_plus_app -- md_preview 2>&1
cargo test -p edit_plus_markdown -- layout 2>&1
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore: workspace check, clippy fixes, cleanup dead code"
```

---

## 验证

所有任务完成后，手动验证：

1. 打开含 BlockQuote 的 md 文件 → 拖选引用文本 → Cmd+C → 粘贴验证内容完整
2. 含 Table 的 md 文件 → 拖选单元格文本 → 复制验证
3. 跨 BlockQuote 和普通段落拖选 → 高亮连续、复制含边界两侧
4. 段落间空白处点击拖拽 → 吸附到最近行，不清除
5. BlockQuote 内双击选词、三击选行正常
6. Shift+方向键在 BlockQuote/Table 内正常扩展
7. Cmd+A 全选含 BlockQuote/Table 全部内容
8. 纯文本 md（无 BlockQuote/Table）选区行为无退行
