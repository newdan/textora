# Find & Replace 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在现有 Find bar 基础上增加 Replace 功能 — inline 同行布局，支持单次替换、全部替换、转义序列和正则表达式。

**Architecture:** 数据模型扩展 SearchState/SearchBarAction；Core 层新增 replace_range 直接操作 gap_buffer；Widget 层重构 paint/event 以支持 replace 模式的输入框和按钮；App 层串联替换流程（替换→重新搜索→跳下一匹配）。

**Tech Stack:** Rust, winit, SIMD search (core::buffer::simd_search), ICU regex (core::buffer::search), rfd (确认对话框)

---

## 文件结构

| 文件 | 职责 | 变更类型 |
|------|------|----------|
| `crates/app/src/search_state.rs` | 新增 replace 相关字段 | 修改 |
| `crates/ui/src/widgets/search_bar.rs` | 重构 paint + event, 新增按钮/输入框 | 修改 |
| `crates/core/src/buffer/edit.rs` | 新增 `replace_range` | 修改 |
| `crates/app/src/app_search.rs` | 新增 replace/replace_all handler | 修改 |
| `crates/app/src/input.rs` | 新增 `EditCommand::FindReplace` + 快捷键映射 | 修改 |
| `crates/app/src/app_dispatch.rs` | 处理新 SearchBarAction 变体 + 对话框 | 修改 |
| `crates/app/src/app_renderer.rs` | 构建 SearchBarSnapshot 时传入新字段 | 修改 |

---

### Task 1: 数据模型扩展

**Files:**
- Modify: `crates/app/src/search_state.rs`
- Modify: `crates/ui/src/widgets/search_bar.rs`

- [ ] **Step 1: SearchState 新增字段**

在 `search_state.rs` 的 `SearchState` struct 中新增 3 个字段：

```rust
// crates/app/src/search_state.rs, 在 pub struct SearchState { ... } 内新增:
    /// Replace text (only relevant when replace mode is active).
    pub replace_query: String,
    /// Whether the replace sub-panel is expanded.
    pub replace_mode: bool,
    /// Keyboard focus: false = find input, true = replace input.
    pub focus_replace: bool,
```

同时更新 `Default` derive — 由于 `String` 和 `bool` 都有 `Default`，derive 自动处理。

- [ ] **Step 2: SearchBarSnapshot 新增字段**

在 `search_bar.rs` 的 `SearchBarSnapshot` struct 中新增 3 个字段：

```rust
// crates/ui/src/widgets/search_bar.rs, 在 pub struct SearchBarSnapshot { ... } 内新增:
    pub replace_query: String,
    pub replace_mode: bool,
    pub focus_replace: bool,
```

- [ ] **Step 3: SearchBarAction 新增变体**

在 `search_bar.rs` 的 `SearchBarAction` enum 中新增 8 个变体：

```rust
// crates/ui/src/widgets/search_bar.rs, 在 pub enum SearchBarAction { ... } 内新增:
    /// 展开/收起 replace 区域
    ToggleReplace,
    /// 切换正则开关
    ToggleRegex,
    /// 替换当前匹配 + 跳到下一匹配
    Replace,
    /// 全部替换
    ReplaceAll,
    /// Tab 到 find 输入框
    FocusFind,
    /// Tab 到 replace 输入框
    FocusReplace,
    /// 插入字符到 replace_query
    InsertReplaceChar(char),
    /// 删除 replace_query 最后一个字符
    ReplaceBackspace,
```

- [ ] **Step 4: SearchState 新增辅助方法**

在 `search_state.rs` 的 `impl SearchState` 中新增 `toggle_replace_mode` 和 `toggle_regex`：

```rust
// crates/app/src/search_state.rs, impl SearchState { ... } 内新增:
    /// Toggle the replace sub-panel. When opening, focus the replace field.
    pub fn toggle_replace_mode(&mut self) {
        self.replace_mode = !self.replace_mode;
        if self.replace_mode {
            self.focus_replace = true; // 展开时自动聚焦 replace 输入框
        } else {
            self.focus_replace = false;
            self.replace_query.clear();
        }
    }

    /// Toggle regex search mode.
    pub fn toggle_regex(&mut self) {
        self.options.use_regex = !self.options.use_regex;
    }
```

- [ ] **Step 5: 更新 SearchState::clear 和 dismiss_or_clear**

在 `clear()` 和 `dismiss_or_clear()` 中清理新字段：

```rust
// clear() 中增加:
        self.replace_query.clear();
        self.replace_mode = false;
        self.focus_replace = false;

// dismiss_or_clear() — 只清空查询时:
        self.replace_query.clear();
        // replace_mode 保持, focus_replace 保持

// dismiss_or_clear() — 关闭面板时 (clear() 负责):
        // clear() 已处理
```

- [ ] **Step 6: 新增和更新单元测试**

在 `search_state.rs` 的 `tests` 模块中添加测试：

```rust
#[test]
fn toggle_replace_mode_expands_and_focuses_replace() {
    let mut state = SearchState::default();
    state.toggle_replace_mode();
    assert!(state.replace_mode);
    assert!(state.focus_replace);
    state.toggle_replace_mode();
    assert!(!state.replace_mode);
    assert!(!state.focus_replace);
    assert!(state.replace_query.is_empty());
}

#[test]
fn toggle_regex_flips_option() {
    let mut state = SearchState::default();
    assert!(!state.options.use_regex);
    state.toggle_regex();
    assert!(state.options.use_regex);
    state.toggle_regex();
    assert!(!state.options.use_regex);
}

#[test]
fn clear_resets_replace_fields() {
    let mut state = SearchState {
        replace_query: "bar".into(),
        replace_mode: true,
        focus_replace: true,
        ..Default::default()
    };
    state.clear();
    assert!(state.replace_query.is_empty());
    assert!(!state.replace_mode);
    assert!(!state.focus_replace);
}
```

- [ ] **Step 7: 运行测试**

```bash
cargo test -p app search_state
```

Expected: 新测试 PASS, 旧测试保持不变。

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/search_state.rs crates/ui/src/widgets/search_bar.rs
git commit -m "feat: extend SearchState and SearchBarAction for replace mode

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 2: Core — replace_range

**Files:**
- Modify: `crates/core/src/buffer/edit.rs`

- [ ] **Step 1: 新增 replace_range 方法**

在 `edit.rs` 的 `impl TextBuffer` 块中新增：

```rust
// crates/core/src/buffer/edit.rs, impl TextBuffer { ... } 内新增:
    /// Delete `range` and insert `replacement` at its start.
    /// Used by find-and-replace to swap a match with replacement text.
    /// The range must be in document byte offsets.
    pub fn replace_range(&mut self, range: std::ops::Range<usize>, replacement: &[u8]) {
        if range.is_empty() && replacement.is_empty() {
            return;
        }
        // Move cursor to start of range
        let start = self.cursor_move_to_offset_internal(self.cursor, range.start);
        // Move end cursor to end of range for deletion
        let end = self.cursor_move_to_offset_internal(start, range.end);

        self.edit_begin(HistoryType::Other, start);
        self.edit_delete(end);
        if !replacement.is_empty() {
            // Temporarily set cursor back to start for write
            self.cursor_move_to_offset_internal(self.cursor, range.start);
            self.edit_write(replacement);
        }
        self.edit_end();
    }
```

- [ ] **Step 2: 运行现有测试确保不破坏**

```bash
cargo test -p core edit
```

Expected: 所有现有测试 PASS。

- [ ] **Step 3: Commit**

```bash
git add crates/core/src/buffer/edit.rs
git commit -m "feat: add TextBuffer::replace_range for find-and-replace

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 3: 转移工具函数

**Files:**
- Create: `crates/app/src/search_escape.rs`

- [ ] **Step 1: 创建转移解析模块**

```rust
// crates/app/src/search_escape.rs
/// Parse C-style escape sequences in a search or replace string.
/// Supported: \n, \t, \r, \\
pub fn parse_escapes(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'n' => { result.push('\n'); i += 2; }
                b't' => { result.push('\t'); i += 2; }
                b'r' => { result.push('\r'); i += 2; }
                b'\\' => { result.push('\\'); i += 2; }
                _ => { result.push('\\'); i += 1; } // unknown escape, keep backslash
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_newline() {
        assert_eq!(parse_escapes(r"hello\nworld"), "hello\nworld");
    }

    #[test]
    fn parse_tab() {
        assert_eq!(parse_escapes(r"a\tb"), "a\tb");
    }

    #[test]
    fn parse_backslash() {
        assert_eq!(parse_escapes(r"path\\to"), "path\\to");
    }

    #[test]
    fn parse_multiple() {
        assert_eq!(parse_escapes(r"line1\nline2\tindented"), "line1\nline2\tindented");
    }

    #[test]
    fn parse_no_escapes() {
        assert_eq!(parse_escapes("plain"), "plain");
    }

    #[test]
    fn parse_empty() {
        assert_eq!(parse_escapes(""), "");
    }

    #[test]
    fn parse_unknown_escape_keeps_backslash() {
        assert_eq!(parse_escapes(r"\x"), "\\x");
    }
}
```

- [ ] **Step 2: 注册模块**

在 `crates/app/src/lib.rs` 中添加：

```rust
// crates/app/src/lib.rs
pub mod search_escape;
```

- [ ] **Step 3: 运行测试**

```bash
cargo test -p app search_escape
```

Expected: 全部 PASS。

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/search_escape.rs crates/app/src/lib.rs
git commit -m "feat: add search escape parsing utility (\n \t \r \\)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 4: SearchBarWidget — paint 重构

**Files:**
- Modify: `crates/ui/src/widgets/search_bar.rs`

这是计划中最大的变更。将 paint 方法拆分为 find-only 和 find+replace 两个子流程。

- [ ] **Step 1: 新增 SearchBarWidget 字段**

在 `SearchBarWidget` struct 中新增：

```rust
// crates/ui/src/widgets/search_bar.rs, SearchBarWidget struct:
    replace_btn_rect: Cell<Rect>,
    replace_all_btn_rect: Cell<Rect>,
    toggle_replace_btn_rect: Cell<Rect>,
    regex_btn_rect: Cell<Rect>,
    focus_replace: bool,
```

在 `new()` 和 `with_snap()` 中初始化：

```rust
    replace_btn_rect: Cell::new(Rect::ZERO),
    replace_all_btn_rect: Cell::new(Rect::ZERO),
    toggle_replace_btn_rect: Cell::new(Rect::ZERO),
    regex_btn_rect: Cell::new(Rect::ZERO),
    focus_replace: false,
```

- [ ] **Step 2: 重构 paint — extract find-only 布局**

将当前 paint 逻辑提取为内部方法 `paint_find_only`，实际 paint 方法调用它或 `paint_find_replace`。

```rust
// crates/ui/src/widgets/search_bar.rs
impl SearchBarWidget {
    /// Paint the find-only bar (current behavior, with new buttons).
    fn paint_find_only(&self, ctx: &mut PaintCtx, dpi: f32, baseline: f32) {
        let pad_left = 36.0 * dpi;
        let icon_x = 12.0 * dpi;
        let pill_w = self.rect.w;
        let pill_x = 0.0;

        // Background + border (same as current)
        let pill_rect = Rect::new(pill_x, 0.0, pill_w, self.rect.h);
        self.pill_rect.set(pill_rect);
        ctx.list.fill(pill_rect, ctx.theme.search_bar_bg);
        ctx.list.fill_rounded(pill_rect, ctx.theme.search_bar_border, 0.0);

        // Icon "/"
        ctx.list.text(pill_x + icon_x, baseline, 14.0 * dpi,
            { let mut c = ctx.theme.search_bar_fg; c[3] *= 0.6; c }, "/");

        // Placeholder or query text
        if self.snap.query.is_empty() && self.snap.preedit_text.is_empty() {
            ctx.list.text(pill_x + pad_left, baseline, 14.0 * dpi,
                { let mut c = ctx.theme.search_bar_fg; c[3] *= 0.4; c }, "Find...");
        }
        if !self.snap.query.is_empty() {
            ctx.list.text(pill_x + pad_left, baseline, 14.0 * dpi,
                ctx.theme.search_bar_fg, &self.snap.query);
        }

        // Right-side buttons: regex, nav, toggle-replace, close
        self.paint_right_buttons(ctx, dpi, baseline, pill_x, pill_w);
    }

    /// Paint the find+replace bar.
    fn paint_find_replace(&self, ctx: &mut PaintCtx, dpi: f32, baseline: f32) {
        let font_size = 14.0 * dpi;
        let pad_left = 36.0 * dpi;
        let icon_x = 12.0 * dpi;
        let pill_w = self.rect.w;
        let pill_x = 0.0;

        // Background + border
        let pill_rect = Rect::new(pill_x, 0.0, pill_w, self.rect.h);
        self.pill_rect.set(pill_rect);
        ctx.list.fill(pill_rect, ctx.theme.search_bar_bg);
        ctx.list.fill_rounded(pill_rect, ctx.theme.search_bar_border, 0.0);

        // Icon "/"
        ctx.list.text(pill_x + icon_x, baseline, font_size,
            { let mut c = ctx.theme.search_bar_fg; c[3] *= 0.6; c }, "/");

        // Estimate right-side buttons total width (to clamp inputs)
        let btn_size = 20.0 * dpi;
        let btn_gap = 4.0 * dpi;
        let right_used = 8.0 * dpi         // pad_right
            + btn_size + btn_gap            // ◀
            + btn_size + btn_gap            // ▶
            + btn_size + btn_gap            // ▲/▼ toggle
            + btn_size;                     // ✕ close
        // + "全部" + "替换" buttons (text, ~2 chars each ~8px wide)
        let replace_btn_w = 2.0 * 8.0 * dpi + 8.0 * dpi; // "替换" ~24px@1x
        let all_btn_w = 2.0 * 8.0 * dpi + 8.0 * dpi;     // "全部" ~24px@1x
        let extra_right = replace_btn_w + btn_gap + all_btn_w + btn_gap
            + (5.0 * 8.0 * dpi) + btn_gap; // match counter "X of Y"
        let total_right = right_used + extra_right;

        // Available width for find+replace inputs
        let input_area_left = pill_x + pad_left;
        let input_area_w = (pill_w - total_right - pad_left).max(100.0 * dpi);
        let sep_w = 16.0 * dpi; // "→" separator width

        // Each input gets roughly half
        let find_w = (input_area_w - sep_w) / 2.0;
        let replace_w = (input_area_w - sep_w) / 2.0;

        // Find input background
        let find_border_color = if self.snap.options_use_regex {
            ctx.theme.sidebar_accent  // regex mode accent
        } else {
            ctx.theme.search_bar_border
        };
        let find_input_rect = Rect::new(input_area_left, (self.rect.h - font_size - 4.0 * dpi) / 2.0,
            find_w, font_size + 4.0 * dpi);
        // (we use text rendering for actual input — just draw bg/border)
        ctx.list.fill_rounded(find_input_rect, {
            let mut c = ctx.theme.search_bar_bg;
            c[0] += 0.05; c[1] += 0.05; c[2] += 0.05;
            c
        }, 3.0 * dpi);
        ctx.list.fill_rounded(find_input_rect, find_border_color, 3.0 * dpi);

        // Find text
        if !self.snap.query.is_empty() {
            ctx.list.text(input_area_left + 4.0 * dpi, baseline, font_size,
                ctx.theme.search_bar_fg, &self.snap.query);
        } else {
            ctx.list.text(input_area_left + 4.0 * dpi, baseline, font_size,
                { let mut c = ctx.theme.search_bar_fg; c[3] *= 0.4; c }, "Find...");
        }

        // Separator "→"
        let sep_x = input_area_left + find_w + 4.0 * dpi;
        ctx.list.text(sep_x, baseline, font_size,
            { let mut c = ctx.theme.search_bar_fg; c[3] *= 0.6; c }, "\u{2192}");

        // Replace input
        let replace_left = sep_x + sep_w;
        let replace_input_rect = Rect::new(replace_left, (self.rect.h - font_size - 4.0 * dpi) / 2.0,
            replace_w, font_size + 4.0 * dpi);
        ctx.list.fill_rounded(replace_input_rect, {
            let mut c = ctx.theme.search_bar_bg;
            c[0] += 0.05; c[1] += 0.05; c[2] += 0.05;
            c
        }, 3.0 * dpi);
        ctx.list.fill_rounded(replace_input_rect, ctx.theme.search_bar_border, 3.0 * dpi);

        // Replace text or placeholder
        if !self.snap.replace_query.is_empty() {
            ctx.list.text(replace_left + 4.0 * dpi, baseline, font_size,
                ctx.theme.search_bar_fg, &self.snap.replace_query);
        } else if self.snap.options_use_regex {
            ctx.list.text(replace_left + 4.0 * dpi, baseline, font_size,
                { let mut c = ctx.theme.search_bar_fg; c[3] *= 0.4; c }, "$1, $2, ...");
        } else {
            ctx.list.text(replace_left + 4.0 * dpi, baseline, font_size,
                { let mut c = ctx.theme.search_bar_fg; c[3] *= 0.4; c }, "替换...");
        }

        // Right-side buttons (same as find-only but includes Replace/All)
        self.paint_right_buttons(ctx, dpi, baseline, pill_x, pill_w);
    }
```

- [ ] **Step 3: 提取右侧按钮布局**

```rust
    /// Paint right-side buttons common to both modes.
    fn paint_right_buttons(&self, ctx: &mut PaintCtx, dpi: f32, baseline: f32,
        pill_x: f32, pill_w: f32)
    {
        let font_size = 14.0 * dpi;
        let btn_size = 20.0 * dpi;
        let pad_right = 8.0 * dpi;
        let btn_gap = 4.0 * dpi;

        let btn_color = { let mut c = ctx.theme.search_bar_fg; c[3] *= 0.6; c };
        let btn_color_hovered = { let mut c = ctx.theme.search_bar_fg; c[3] *= 0.9; c };
        let btn_clr = |hovered: bool| {
            if hovered { btn_color_hovered } else { btn_color }
        };
        let btn_bg_clr = |hovered: bool| {
            if hovered {
                Some(ctx.theme.menu_hover)
            } else {
                None
            }
        };

        // Layout from right to left
        let mut right_cursor = pill_x + pill_w - pad_right;

        // Close button ✕ (always present)
        {
            let cx = right_cursor - btn_size * 0.5;
            let cy = self.rect.h * 0.5;
            let cr = Rect::new(cx - btn_size * 0.5, cy - btn_size * 0.5, btn_size, btn_size);
            self.clear_btn_rect.set(cr);
            let is_hover = self.hovered_btn == HoveredButton::CloseBar;
            if is_hover { ctx.list.fill_rounded(cr, ctx.theme.menu_hover, 4.0 * dpi); }
            ctx.list.text(cx - 4.0 * dpi, baseline, font_size, btn_clr(is_hover), "\u{2715}");
            right_cursor -= btn_size + btn_gap;
        }

        // Toggle replace ▼/▲
        {
            let cx = right_cursor - btn_size * 0.5;
            let cy = self.rect.h * 0.5;
            let cr = Rect::new(cx - btn_size * 0.5, cy - btn_size * 0.5, btn_size, btn_size);
            self.toggle_replace_btn_rect.set(cr);
            let is_hover = self.hovered_btn == HoveredButton::ToggleReplace;
            if is_hover { ctx.list.fill_rounded(cr, ctx.theme.menu_hover, 4.0 * dpi); }
            let arrow = if self.snap.replace_mode { "\u{25b2}" } else { "\u{25bc}" };
            ctx.list.text(cx - 4.0 * dpi, baseline, font_size, btn_clr(is_hover), arrow);
            right_cursor -= btn_size + btn_gap;
        }

        // Navigation ◀ ▶ and match count (only when query non-empty)
        if !self.snap.query.is_empty() {
            if self.snap.match_count > 0 {
                let current = self.snap.current_match.saturating_add(1).min(self.snap.match_count);
                let count_text = format!("{}/{}", current, self.snap.match_count);
                let count_w = count_text.len() as f32 * 8.0 * dpi;
                right_cursor -= count_w + btn_gap;

                // ▶ next
                {
                    let cx = right_cursor - btn_size * 0.5;
                    let cy = self.rect.h * 0.5;
                    self.next_btn_rect.set(Rect::new(cx - btn_size * 0.5, cy - btn_size * 0.5, btn_size, btn_size));
                    let is_hover = self.hovered_btn == HoveredButton::Next;
                    if is_hover { ctx.list.fill_rounded(self.next_btn_rect.get(), ctx.theme.menu_hover, 4.0 * dpi); }
                    ctx.list.text(cx - 4.0 * dpi, baseline, font_size, btn_clr(is_hover), "\u{25b6}");
                    right_cursor -= btn_size + btn_gap;
                }

                // ◀ prev
                {
                    let cx = right_cursor - btn_size * 0.5;
                    let cy = self.rect.h * 0.5;
                    self.prev_btn_rect.set(Rect::new(cx - btn_size * 0.5, cy - btn_size * 0.5, btn_size, btn_size));
                    let is_hover = self.hovered_btn == HoveredButton::Prev;
                    if is_hover { ctx.list.fill_rounded(self.prev_btn_rect.get(), ctx.theme.menu_hover, 4.0 * dpi); }
                    ctx.list.text(cx - 4.0 * dpi, baseline, font_size, btn_clr(is_hover), "\u{25c0}");
                    right_cursor -= btn_size + btn_gap;
                }

                // Match counter text
                ctx.list.text(right_cursor - count_w + btn_gap, baseline, font_size,
                    ctx.theme.search_bar_fg, &count_text);
                right_cursor -= count_w + btn_gap;
            } else {
                // No results text
                let no_res = "No results";
                let no_w = no_res.len() as f32 * 8.0 * dpi;
                ctx.list.text(right_cursor - no_w, baseline, font_size,
                    ctx.theme.search_bar_no_results_fg, no_res);
                right_cursor -= no_w + btn_gap;
            }
        }

        // Replace All button "全部" (only in replace mode)
        if self.snap.replace_mode {
            let all_text = "全部";
            let all_w = 4.0 * 8.0 * dpi; // ~32px for 2 CJK chars + padding
            right_cursor -= all_w + btn_gap;
            let all_rect = Rect::new(right_cursor, (self.rect.h - font_size - 4.0 * dpi) / 2.0,
                all_w, font_size + 4.0 * dpi);
            self.replace_all_btn_rect.set(all_rect);
            let is_hover = self.hovered_btn == HoveredButton::ReplaceAll;
            let disabled = self.snap.match_count == 0;
            let clr = if disabled {
                { let mut c = ctx.theme.search_bar_fg; c[3] *= 0.3; c }
            } else {
                btn_clr(is_hover)
            };
            if is_hover && !disabled {
                ctx.list.fill_rounded(all_rect, ctx.theme.menu_hover, 3.0 * dpi);
            }
            ctx.list.text(right_cursor + 4.0 * dpi, baseline, font_size, clr, all_text);

            // Replace button "替换"
            let rep_text = "替换";
            let rep_w = 4.0 * 8.0 * dpi;
            right_cursor -= rep_w + btn_gap;
            let rep_rect = Rect::new(right_cursor, (self.rect.h - font_size - 4.0 * dpi) / 2.0,
                rep_w, font_size + 4.0 * dpi);
            self.replace_btn_rect.set(rep_rect);
            let is_hover = self.hovered_btn == HoveredButton::Replace;
            if is_hover && !disabled {
                ctx.list.fill_rounded(rep_rect, ctx.theme.menu_hover, 3.0 * dpi);
            }
            ctx.list.text(right_cursor + 4.0 * dpi, baseline, font_size, clr, rep_text);

            right_cursor -= btn_gap;
        }

        // Regex toggle .* (only when not in replace mode, or in replace mode at specific position)
        {
            let cx = right_cursor - btn_size * 0.5;
            let cy = self.rect.h * 0.5;
            let cr = Rect::new(cx - btn_size * 0.5, cy - btn_size * 0.5, btn_size, btn_size);
            self.regex_btn_rect.set(cr);
            let is_hover = self.hovered_btn == HoveredButton::Regex;
            let is_active = self.snap.options_use_regex;
            let clr = if is_active { ctx.theme.sidebar_accent } else { btn_clr(is_hover) };
            if is_hover { ctx.list.fill_rounded(cr, ctx.theme.menu_hover, 4.0 * dpi); }
            ctx.list.text(cx - 4.0 * dpi, baseline, font_size, clr, ".*");
        }
    }
```

- [ ] **Step 4: 更新主 paint 方法**

```rust
    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 || !self.snap.visible {
            return;
        }
        let dpi = ctx.dpi;
        let font_size = 14.0 * dpi;
        let baseline = self.rect.h * 0.5 + font_size * 0.35;

        if self.snap.replace_mode {
            self.paint_find_replace(ctx, dpi, baseline);
        } else {
            self.paint_find_only(ctx, dpi, baseline);
        }

        // Cursor (blinking)
        if self.snap.blink_on {
            let cursor_h = font_size;
            let cursor_w = 2.0 * dpi;
            let pad_left = 36.0 * dpi;
            let cursor_y = baseline - cursor_h * 0.75;
            let cursor_rect = Rect::new(pad_left + self.snap.cursor_x, cursor_y, cursor_w, cursor_h);
            ctx.list.fill(cursor_rect, ctx.theme.search_bar_fg);
        }
    }
```

- [ ] **Step 5: 更新 HoveredButton enum**

```rust
#[derive(Copy, Clone, PartialEq, Eq)]
enum HoveredButton {
    None,
    CloseBar,        // was Clear
    Prev,
    Next,
    ToggleReplace,
    Regex,
    Replace,
    ReplaceAll,
}
```

- [ ] **Step 6: 更新 Snapshot 结构以包含 options**

因为 regex 按钮需要知道 `use_regex` 状态，需要在 `SearchBarSnapshot` 中新增：

```rust
    pub options_use_regex: bool,
```

- [ ] **Step 7: 运行现有测试并修复 breakage**

```bash
cargo test -p ui search_bar
```

由于搜索栏的绘制逻辑变了，一些旧测试会失败（例如测试检查特定 cmd 索引位置）。逐个更新测试以匹配新布局。

- [ ] **Step 8: Commit**

```bash
git add crates/ui/src/widgets/search_bar.rs
git commit -m "feat: refactor SearchBarWidget paint for replace mode

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 5: SearchBarWidget — event 重构

**Files:**
- Modify: `crates/ui/src/widgets/search_bar.rs`

- [ ] **Step 1: 更新 on_event — 新增 Tab 和替换事件处理**

```rust
    fn on_event(&mut self, ev: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
        if !self.snap.visible {
            return None;
        }
        match ev {
            Event::KeyDown(kc) => match kc {
                KeyCode::Escape => Some(WidgetAction::SearchBar(SearchBarAction::DismissOrClear)),
                KeyCode::Enter => {
                    if self.snap.replace_mode && self.snap.focus_replace {
                        // Enter in replace field -> Replace
                        Some(WidgetAction::SearchBar(SearchBarAction::Replace))
                    } else {
                        // Enter in find field -> Next
                        Some(WidgetAction::SearchBar(SearchBarAction::Next))
                    }
                }
                KeyCode::Tab => {
                    if self.snap.replace_mode {
                        if self.snap.focus_replace {
                            Some(WidgetAction::SearchBar(SearchBarAction::FocusFind))
                        } else {
                            Some(WidgetAction::SearchBar(SearchBarAction::FocusReplace))
                        }
                    } else {
                        None // Tab ignored when replace mode off
                    }
                }
                KeyCode::Backspace => {
                    if self.snap.replace_mode && self.snap.focus_replace {
                        // Backspace in replace field
                        Some(WidgetAction::SearchBar(SearchBarAction::ReplaceBackspace))
                    } else {
                        Some(WidgetAction::SearchBar(SearchBarAction::Backspace))
                    }
                }
                KeyCode::Char(c) => {
                    if self.snap.replace_mode && self.snap.focus_replace {
                        Some(WidgetAction::SearchBar(SearchBarAction::InsertReplaceChar(*c)))
                    } else {
                        Some(WidgetAction::SearchBar(SearchBarAction::InsertChar(*c)))
                    }
                }
                _ => None,
            },
            Event::MouseMove { px, py } => {
                let old = self.hovered_btn;
                self.hovered_btn = HoveredButton::None;
                self.update_hover(*px, *py);
                if self.hovered_btn != HoveredButton::None {
                    _ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                }
                if old != self.hovered_btn {
                    Some(WidgetAction::Consumed)
                } else {
                    None
                }
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                self.handle_mouse_down(*px, *py)
            }
            _ => None,
        }
    }

    fn update_hover(&mut self, px: f32, py: f32) {
        let check = |r: &Rect| r.w > 0.0 && r.contains(px, py);
        if check(&self.clear_btn_rect.get()) { self.hovered_btn = HoveredButton::CloseBar; return; }
        if check(&self.toggle_replace_btn_rect.get()) { self.hovered_btn = HoveredButton::ToggleReplace; return; }
        if check(&self.regex_btn_rect.get()) { self.hovered_btn = HoveredButton::Regex; return; }
        if check(&self.prev_btn_rect.get()) { self.hovered_btn = HoveredButton::Prev; return; }
        if check(&self.next_btn_rect.get()) { self.hovered_btn = HoveredButton::Next; return; }
        if check(&self.replace_btn_rect.get()) { self.hovered_btn = HoveredButton::Replace; return; }
        if check(&self.replace_all_btn_rect.get()) { self.hovered_btn = HoveredButton::ReplaceAll; return; }
    }

    fn handle_mouse_down(&self, px: f32, py: f32) -> Option<WidgetAction> {
        let check = |r: &Rect| r.w > 0.0 && r.contains(px, py);
        if check(&self.clear_btn_rect.get()) {
            return Some(WidgetAction::SearchBar(SearchBarAction::Close));
        }
        if check(&self.toggle_replace_btn_rect.get()) {
            return Some(WidgetAction::SearchBar(SearchBarAction::ToggleReplace));
        }
        if check(&self.regex_btn_rect.get()) {
            return Some(WidgetAction::SearchBar(SearchBarAction::ToggleRegex));
        }
        if check(&self.prev_btn_rect.get()) {
            return Some(WidgetAction::SearchBar(SearchBarAction::Prev));
        }
        if check(&self.next_btn_rect.get()) {
            return Some(WidgetAction::SearchBar(SearchBarAction::Next));
        }
        if self.snap.match_count > 0 {
            if check(&self.replace_btn_rect.get()) {
                return Some(WidgetAction::SearchBar(SearchBarAction::Replace));
            }
            if check(&self.replace_all_btn_rect.get()) {
                return Some(WidgetAction::SearchBar(SearchBarAction::ReplaceAll));
            }
        }
        None
    }
```

- [ ] **Step 2: InsertReplaceChar 和 ReplaceBackspace 已在 T1 中定义**

无需额外操作。这两个变体在 T1 Step 3 的 enum 中已包含。

- [ ] **Step 3: 构建并修复测试**

```bash
cargo test -p ui search_bar
```

- [ ] **Step 4: Commit**

```bash
git add crates/ui/src/widgets/search_bar.rs
git commit -m "feat: refactor SearchBarWidget events for replace mode

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 6: App 层 — 处理替换操作

**Files:**
- Modify: `crates/app/src/app_search.rs`

- [ ] **Step 1: 重写 apply_search_bar_action 处理新变体**

```rust
// crates/app/src/app_search.rs
impl App {
    pub(crate) fn apply_search_bar_action(&mut self, action: &SearchBarAction) -> bool {
        use SearchBarAction as SA;
        let needs_search = matches!(action,
            SA::InsertChar(_) | SA::Backspace | SA::ClearQuery
            | SA::InsertReplaceChar(_) | SA::ReplaceBackspace
        );
        let should_scroll = matches!(action, SA::Next | SA::Prev);
        let redirected_to_replace = matches!(action, SA::ToggleReplace | SA::Replace | SA::ReplaceAll
            | SA::ToggleRegex | SA::FocusFind | SA::FocusReplace
            | SA::InsertReplaceChar(_) | SA::ReplaceBackspace);

        if let Some(dv) = self.workspace.doc_views.get_mut(self.workspace.active_index) {
            match action {
                SA::InsertChar(c) => {
                    dv.search_state.query.push(*c);
                    dv.search_state.set_cursor_byte_pos(dv.search_state.query.len());
                    dv.cursor_render_state.cursor_blink_instant = std::time::Instant::now();
                }
                SA::Backspace => {
                    dv.search_state.query.pop();
                    dv.search_state.set_cursor_byte_pos(dv.search_state.query.len());
                    dv.cursor_render_state.cursor_blink_instant = std::time::Instant::now();
                }
                SA::InsertReplaceChar(c) => {
                    dv.search_state.replace_query.push(*c);
                    dv.cursor_render_state.cursor_blink_instant = std::time::Instant::now();
                }
                SA::ReplaceBackspace => {
                    dv.search_state.replace_query.pop();
                    dv.cursor_render_state.cursor_blink_instant = std::time::Instant::now();
                }
                SA::Next => dv.search_state.next_match(),
                SA::Prev => dv.search_state.prev_match(),
                SA::Close => {
                    dv.search_state.panel_visible = false;
                    dv.search_state.query.clear();
                    dv.search_state.matches.clear();
                    dv.search_state.active_match_idx = 0;
                    dv.search_state.replace_query.clear();
                    dv.search_state.replace_mode = false;
                    dv.search_state.focus_replace = false;
                    dv.search_state.set_cursor_byte_pos(0);
                }
                SA::DismissOrClear => dv.search_state.dismiss_or_clear(),
                SA::ClearQuery => {
                    dv.search_state.query.clear();
                    dv.search_state.matches.clear();
                    dv.search_state.active_match_idx = 0;
                    dv.search_state.set_cursor_byte_pos(0);
                }
                SA::MoveCursor(pos) => {
                    dv.search_state.set_cursor_byte_pos(*pos);
                }
                SA::ToggleReplace => {
                    dv.search_state.toggle_replace_mode();
                }
                SA::ToggleRegex => {
                    dv.search_state.toggle_regex();
                }
                SA::FocusFind => {
                    dv.search_state.focus_replace = false;
                }
                SA::FocusReplace => {
                    dv.search_state.focus_replace = true;
                }
                SA::Replace => {
                    self.perform_single_replace();
                    return true; // triggers re-search
                }
                SA::ReplaceAll => {
                    self.perform_replace_all();
                    return true;
                }
            }
            if should_scroll {
                self.scroll_to_active_match();
            }
        }
        needs_search
    }
```

- [ ] **Step 2: 实现 perform_single_replace**

```rust
    /// Replace the current match with replace_query and advance to next match.
    fn perform_single_replace(&mut self) {
        if let Some(dv) = self.workspace.doc_views.get_mut(self.workspace.active_index) {
            let Some(range) = dv.search_state.active_match() else { return; };
            let replacement = crate::search_escape::parse_escapes(&dv.search_state.replace_query);

            if dv.search_state.options.use_regex {
                // Use ICU regex replace
                let pattern = crate::search_escape::parse_escapes(&dv.search_state.query);
                let _ = dv.tb.find_and_replace(
                    &pattern,
                    dv.search_state.options,
                    replacement.as_bytes(),
                );
            } else {
                dv.tb.replace_range(range, replacement.as_bytes());
            }
            dv.set_dirty(true);
        }
    }
```

- [ ] **Step 3: 实现 perform_replace_all**

```rust
    /// Replace all matches. Called from AppAction handler after confirmation.
    fn perform_replace_all(&mut self) {
        if let Some(dv) = self.workspace.doc_views.get_mut(self.workspace.active_index) {
            let count = dv.search_state.matches.len();
            if count == 0 { return; }
            let replacement = crate::search_escape::parse_escapes(&dv.search_state.replace_query);

            if dv.search_state.options.use_regex {
                let pattern = crate::search_escape::parse_escapes(&dv.search_state.query);
                let _ = dv.tb.find_and_replace_all(
                    &pattern,
                    dv.search_state.options,
                    replacement.as_bytes(),
                );
            } else {
                // Replace in reverse order to preserve offsets
                let matches: Vec<_> = dv.search_state.matches.clone();
                dv.tb.edit_begin_grouping();
                for range in matches.into_iter().rev() {
                    dv.tb.replace_range(range, replacement.as_bytes());
                }
                dv.tb.edit_end_grouping();
            }
            dv.set_dirty(true);
            // Re-search to update match count
            self.perform_search_for_active_doc();
        }
    }

    /// Get the count of matches for the current search (for confirmation dialog).
    pub(crate) fn current_match_count(&self) -> usize {
        self.workspace.doc_views
            .get(self.workspace.active_index)
            .map(|dv| dv.search_state.matches.len())
            .unwrap_or(0)
    }
```

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/app_search.rs
git commit -m "feat: implement single replace and replace all in app layer

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 7: 快捷键映射

**Files:**
- Modify: `crates/app/src/input.rs`

- [ ] **Step 1: 新增 EditCommand::FindReplace**

```rust
// crates/app/src/input.rs, EditCommand enum 中新增:
    /// Open find bar in replace mode
    FindReplace,
```

- [ ] **Step 2: 映射 Cmd+Shift+F 和 Cmd+Opt+F**

在 `key_to_command` 函数中添加映射。找到处理 `"f"` 键的位置 (`ctrl || super_` 的分支):

```rust
// crates/app/src/input.rs, key_to_command, 在 "f" 的 case 附近:
                // Cmd+Shift+F or Cmd+Opt+F -> FindReplace
                let any_find_replace = key == "f" && super_ && (shift || alt);
                if any_find_replace {
                    return Some(EditCommand::FindReplace);
                }
```

- [ ] **Step 3: 在 handle_command 中处理 FindReplace**

```rust
// crates/app/src/app_dispatch.rs, handle_command, 在 EditCommand::Find 的 case 后:
            EditCommand::FindReplace => {
                if let Some(dv) = self.workspace.doc_views.get_mut(self.workspace.active_index) {
                    dv.search_state.panel_visible = true;
                    // 展开 replace 模式（若未展开）
                    if !dv.search_state.replace_mode {
                        dv.search_state.toggle_replace_mode();
                    }
                    // 聚焦 replace 输入框
                    dv.search_state.focus_replace = true;
                    self.needs_redraw = true;
                }
                return;
            }
```

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/input.rs crates/app/src/app_dispatch.rs
git commit -m "feat: add Cmd+Shift+F / Cmd+Opt+F shortcut for find+replace

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 8: 确认对话框 + App dispatch 接入

**Files:**
- Modify: `crates/app/src/app_dispatch.rs`

- [ ] **Step 1: 替换 All 确认对话框**

在 `dispatch` 方法的 `AppAction::SearchBarAction(SearchBarAction::ReplaceAll)` 分支中，改为弹出对话框：

```rust
// crates/app/src/app_dispatch.rs, dispatch 方法中:
            AppAction::SearchBarAction(action) => {
                // 对 ReplaceAll 需要先确认
                if matches!(&action, SearchBarAction::ReplaceAll) {
                    let count = self.current_match_count();
                    if count == 0 {
                        self.needs_redraw = true;
                        return;
                    }
                    let query = self.workspace.doc_views
                        .get(self.workspace.active_index)
                        .map(|dv| dv.search_state.query.clone())
                        .unwrap_or_default();
                    let replace = self.workspace.doc_views
                        .get(self.workspace.active_index)
                        .map(|dv| dv.search_state.replace_query.clone())
                        .unwrap_or_default();
                    let msg = format!(
                        "Replace all {} occurrences of \"{}\" with \"{}\"?",
                        count, query, replace
                    );
                    let confirmed = rfd::MessageDialog::new()
                        .set_title("Replace All")
                        .set_description(&msg)
                        .set_buttons(rfd::MessageButtons::OkCancel)
                        .show();
                    if !confirmed {
                        self.needs_redraw = true;
                        return;
                    }
                }
                let needs_search = self.apply_search_bar_action(&action);
                if needs_search {
                    self.perform_search_for_active_doc();
                }
                self.needs_redraw = true;
            }
```

- [ ] **Step 2: 确保 rfd 依赖可用**

检查 `crates/app/Cargo.toml` 中 `rfd = "0.15"` 存在（已确认存在）。

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/app_dispatch.rs
git commit -m "feat: add confirmation dialog for Replace All

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 9: 管线接入 — 构建 SearchBarSnapshot

**Files:**
- Modify: `crates/app/src/app_renderer.rs`

- [ ] **Step 1: 在构建 SearchBarSnapshot 时传入新字段**

在 `app_renderer.rs` 的 `build_shell_inputs` → `set_search_input` 附近：

```rust
// crates/app/src/app_renderer.rs, 在 SearchBarSnapshot 构造处:
                self.ui_shell.set_search_input(SearchBarSnapshot {
                    query: search.query.clone(),
                    preedit_text: if search.panel_visible { self.preedit_text.clone() } else { String::new() },
                    match_count: search.matches.len(),
                    current_match: search.active_match_idx,
                    visible: search.panel_visible,
                    cursor_x: 0.0,
                    blink_on,
                    replace_query: search.replace_query.clone(),     // NEW
                    replace_mode: search.replace_mode,               // NEW
                    focus_replace: search.focus_replace,             // NEW
                    options_use_regex: search.options.use_regex,     // NEW
                });
```

- [ ] **Step 2: Build check**

```bash
cargo build
```

Expected: 编译通过（可能需要在 `SearchBarSnapshot` 中先加入 `options_use_regex` 字段，已在 Task 4 Step 6 完成）。

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/app_renderer.rs
git commit -m "feat: wire new replace fields into SearchBarSnapshot

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 10: 全量构建 + 冒烟测试

**Files:**
- No new files

- [ ] **Step 1: 全量编译**

```bash
cargo build 2>&1
```

Expected: 编译成功，无警告。

- [ ] **Step 2: 运行全部测试**

```bash
cargo test 2>&1
```

Expected: 全部测试 PASS。

- [ ] **Step 3: 运行冒烟测试**

```bash
cargo test --test render_smoke 2>&1
```

Expected: 冒烟测试 PASS。

- [ ] **Step 4: Commit (若有 fixup)**

```bash
git commit -m "chore: fixup after full build and test

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## 自审清单

1. **Spec coverage**: 每个 spec 需求都对应到具体 task — 数据模型 (T1), UI (T4, T5), 单次替换 (T6), 全部替换 (T6, T8), 转义 (T3), 正则 (T4, T6), 快捷键 (T7), 确认对话框 (T8), 主题色 (T4 中使用 `sidebar_accent`)。
2. **Placeholder scan**: 无 TBD/TODO/模糊描述。
3. **Type consistency**: `SearchBarAction` 变体在 T1 定义，T5 产出，T6 消费。`SearchBarSnapshot` 字段在 T1 新增，T9 赋值，T4 使用。`replace_range` 在 T2 新增，T6 调用。

## 推迟项

以下 spec 需求推迟到后续 PR：

- **Tooltip (hover 400ms 后显示)**: 需要 hover 计时器 + overlay 弹出层。当前计划中 T4/T5 内暂不包含，用 `cursor_hint` 作为替代。后续可作为独立 PR 增加 tooltip overlay 系统。
