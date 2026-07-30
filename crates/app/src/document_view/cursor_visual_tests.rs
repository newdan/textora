#![allow(deprecated)]
use super::*;
use crate::snap_tree::DisplayLineEntry;

const TEST_LINE_HEIGHT: f32 = 24.27;

// ── line_byte_offset accessor ──────────────────────────────────────

#[test]
fn line_byte_offset_first_line() {
    let dv = DocumentView::new(vec!["hello".to_string(), "world".to_string()], 10, 10.0);
    assert_eq!(dv.line_byte_offset(0), Some(0));
}

#[test]
fn line_byte_offset_second_line() {
    // "hello" = 5 bytes + 1 newline = 6
    let dv = DocumentView::new(vec!["hello".to_string(), "world".to_string()], 10, 10.0);
    assert_eq!(dv.line_byte_offset(1), Some(6));
}

#[test]
fn line_byte_offset_out_of_range() {
    let dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    assert_eq!(dv.line_byte_offset(99), None);
}

#[test]
fn line_byte_offset_empty_doc() {
    let dv = DocumentView::new(vec![], 10, 10.0);
    assert_eq!(dv.line_byte_offset(0), Some(0));
}

#[test]
fn line_byte_offset_cjk() {
    // "e4b896" = 3 bytes, "e7958c" = 3 bytes, joined by newline = 7 bytes total
    let dv = DocumentView::new(vec!["世".to_string(), "界".to_string()], 10, 10.0);
    assert_eq!(dv.line_byte_offset(0), Some(0));
    assert_eq!(dv.line_byte_offset(1), Some(4)); // 3 + 1 newline
}

// ── ensure_cursor_visible scrolls correctly ───────────────────────

#[test]
fn ensure_cursor_visible_scrolls_down() {
    let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
    let mut dv = DocumentView::new(lines, 10, 10.0);
    // viewport shows lines 0..10
    assert_eq!(
        dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count()),
        0..10
    );

    // Move cursor to line 50 (way off-screen)
    let offset = dv.line_byte_offset(50).unwrap();
    dv.cursor_move_to_offset(offset);
    // Autoscroll: ensure cursor line is visible
    {
        let line = dv.cursor_line();
        let range = dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count());
        if line < range.start {
            dv.presentation.display.viewport.scroll_to_doc_line(line);
        } else if line >= range.end {
            let target = line.saturating_sub(dv.presentation.display.viewport.visible_rows - 1);
            dv.presentation.display.viewport.scroll_to_doc_line(target);
        }
    }

    // Viewport should scroll so line 50 is visible
    let range = dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count());
    assert!(range.start <= 50 && range.end > 50, "line 50 should be visible in range {range:?}");
}

#[test]
fn ensure_cursor_visible_scrolls_up() {
    let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
    let mut dv = DocumentView::new(lines, 10, 10.0);
    // Scroll to middle
    dv.presentation.display.viewport.scroll_to_doc_line(50);
    assert_eq!(
        dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count()),
        50..60
    );

    // Move cursor to line 3 (off-screen above)
    let offset = dv.line_byte_offset(3).unwrap();
    dv.cursor_move_to_offset(offset);
    // Autoscroll: ensure cursor line is visible
    {
        let line = dv.cursor_line();
        let range = dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count());
        if line < range.start {
            dv.presentation.display.viewport.scroll_to_doc_line(line);
        } else if line >= range.end {
            let target = line.saturating_sub(dv.presentation.display.viewport.visible_rows - 1);
            dv.presentation.display.viewport.scroll_to_doc_line(target);
        }
    }

    let range = dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count());
    assert!(range.start <= 3 && range.end > 3, "line 3 should be visible in range {range:?}");
}

#[test]
fn ensure_cursor_visible_no_scroll_when_visible() {
    let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
    let mut dv = DocumentView::new(lines, 10, 10.0);
    let range_before =
        dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count());

    // Move cursor to line 5 (within visible range 0..10)
    let offset = dv.line_byte_offset(5).unwrap();
    dv.cursor_move_to_offset(offset);
    // Autoscroll: ensure cursor line is visible
    {
        let line = dv.cursor_line();
        let range = dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count());
        if line < range.start {
            dv.presentation.display.viewport.scroll_to_doc_line(line);
        } else if line >= range.end {
            let target = line.saturating_sub(dv.presentation.display.viewport.visible_rows - 1);
            dv.presentation.display.viewport.scroll_to_doc_line(target);
        }
    }

    assert_eq!(
        dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count()),
        range_before,
        "should not scroll when cursor is already visible"
    );
}

#[test]
fn ensure_cursor_visible_at_eof() {
    let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
    let mut dv = DocumentView::new(lines, 10, 10.0);

    // Move cursor to last line
    let offset = dv.line_byte_offset(99).unwrap();
    dv.cursor_move_to_offset(offset);
    // Autoscroll: ensure cursor line is visible
    {
        let line = dv.cursor_line();
        let range = dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count());
        if line < range.start {
            dv.presentation.display.viewport.scroll_to_doc_line(line);
        } else if line >= range.end {
            let target = line.saturating_sub(dv.presentation.display.viewport.visible_rows - 1);
            dv.presentation.display.viewport.scroll_to_doc_line(target);
        }
    }

    let range = dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count());
    assert!(range.start <= 99 && range.end >= 99, "last line should be visible in range {range:?}");
}

// ── cursor_line / cursor_column after movement ────────────────────

#[test]
fn cursor_line_after_move_to_offset() {
    let dv =
        DocumentView::new(vec!["aaa".to_string(), "bbb".to_string(), "ccc".to_string()], 10, 10.0);
    // "aaa" = 3 bytes + 1 newline = 4 bytes per line, so line 2 starts at offset 8
    let mut dv = dv;
    dv.cursor_move_to_offset(9); // 'b' of "bbb" on line 1... actually offset 8 is 'b' of line 2
    assert_eq!(dv.cursor_line(), 2);
    assert_eq!(dv.cursor_column(), 1);
}

#[test]
fn cursor_column_at_line_start() {
    let mut dv = DocumentView::new(vec!["hello".to_string(), "world".to_string()], 10, 10.0);
    dv.cursor_move_to_offset(6); // start of "world"
    assert_eq!(dv.cursor_line(), 1);
    assert_eq!(dv.cursor_column(), 0);
}

#[test]
fn cursor_column_at_line_end() {
    let mut dv = DocumentView::new(vec!["hello".to_string(), "world".to_string()], 10, 10.0);
    dv.cursor_move_to_offset(5); // end of "hello" (before newline)
    assert_eq!(dv.cursor_line(), 0);
    assert_eq!(dv.cursor_column(), 5);
}

// ── visible_line preserves content for hit-test ───────────────────

#[test]
fn visible_line_matches_source_for_mixed_content() {
    let lines = vec!["abc  def".to_string(), "hello\tworld".to_string(), "62220099".to_string()];
    let dv = DocumentView::new(lines, 10, 10.0);
    assert_eq!(dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap(), &b"abc  def"[..]);
    assert_eq!(
        dv.visible_line_with_line_height(1, TEST_LINE_HEIGHT).unwrap(),
        &b"hello\tworld"[..]
    );
    assert_eq!(dv.visible_line_with_line_height(2, TEST_LINE_HEIGHT).unwrap(), &b"62220099"[..]);
}

// ── viewport visible_rows vs line_count ───────────────────────────

#[test]
fn viewport_visible_rows_clamped_to_line_count() {
    let lines: Vec<String> = (0..5).map(|i| format!("line {i}")).collect();
    let dv = DocumentView::new(lines, 50, 10.0);
    assert_eq!(
        dv.visible_line_count_with_line_height(TEST_LINE_HEIGHT),
        5,
        "visible_line_count should not exceed total lines"
    );
}

#[test]
fn viewport_scroll_preserves_line_count() {
    let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
    let mut dv = DocumentView::new(lines, 10, 10.0);
    assert_eq!(dv.line_count(), 100);
    dv.presentation.display.viewport.scroll_to_doc_line(50);
    assert_eq!(dv.line_count(), 100, "line_count should not change on scroll");
}

// ── line_byte_length accessor ─────────────────────────────────────

#[test]
fn line_byte_length_first_line() {
    let dv = DocumentView::new(vec!["hello".to_string(), "world".to_string()], 10, 10.0);
    // "hello" = 5 bytes (newline not included in line_lengths)
    assert_eq!(dv.line_byte_length(0), Some(5));
}

#[test]
fn line_byte_length_last_line() {
    let dv = DocumentView::new(vec!["hello".to_string(), "world".to_string()], 10, 10.0);
    // "world" = 5 bytes (no trailing newline on last line)
    assert_eq!(dv.line_byte_length(1), Some(5));
}

#[test]
fn line_byte_length_out_of_range() {
    let dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    assert_eq!(dv.line_byte_length(99), None);
}

#[test]
fn line_byte_length_empty_doc() {
    let dv = DocumentView::new(vec![], 10, 10.0);
    assert_eq!(dv.line_byte_length(0), Some(0));
}

#[test]
fn line_byte_length_cjk() {
    let dv = DocumentView::new(vec!["世界".to_string(), "你好".to_string()], 10, 10.0);
    // "世界" = 6 bytes (newline not included)
    assert_eq!(dv.line_byte_length(0), Some(6));
    // "你好" = 6 bytes (last line, no newline)
    assert_eq!(dv.line_byte_length(1), Some(6));
}

// ── viewport scroll_to for visual-line-aware scrolling ────────────

#[test]
fn viewport_scroll_to_preserves_visual_line_awareness() {
    // Simulate the logic from shape_visible_lines:
    // cursor_doc_line in viewport but cursor_visual_line >= visible_rows
    let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
    let mut dv = DocumentView::new(lines, 10, 10.0);
    // viewport: 0..10, visible_rows=10

    // Simulate cursor on doc line 15, visual line 12 (>= visible_rows=10)
    let cursor_doc_line = 15usize;
    let visible_rows = dv.presentation.display.viewport.visible_rows;
    assert_eq!(visible_rows, 10);

    // The scroll logic from shape_visible_lines:
    dv.presentation
        .display
        .viewport
        .scroll_to_doc_line(cursor_doc_line.saturating_sub(visible_rows.saturating_sub(1)));
    let range = dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count());
    assert!(
        range.start <= cursor_doc_line && range.end > cursor_doc_line,
        "cursor doc line {cursor_doc_line} should be visible in range {range:?}"
    );
}

#[test]
fn viewport_scroll_to_cursor_above_viewport() {
    let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
    let mut dv = DocumentView::new(lines, 10, 10.0);
    dv.presentation.display.viewport.scroll_to_doc_line(50);
    assert_eq!(
        dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count()),
        50..60
    );

    // Cursor doc line is above viewport
    let cursor_doc_line = 3usize;
    let range = dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count());
    assert!(cursor_doc_line < range.start);

    // Scroll logic: scroll to cursor_doc_line
    dv.presentation.display.viewport.scroll_to_doc_line(cursor_doc_line);
    let range = dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count());
    assert!(
        range.start <= cursor_doc_line && range.end > cursor_doc_line,
        "cursor doc line {cursor_doc_line} should be visible in range {range:?}"
    );
}

#[test]
fn viewport_scroll_to_cursor_below_viewport() {
    let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
    let mut dv = DocumentView::new(lines, 10, 10.0);
    assert_eq!(
        dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count()),
        0..10
    );

    // Cursor doc line is below viewport
    let cursor_doc_line = 50usize;
    let visible_rows = dv.presentation.display.viewport.visible_rows;

    // Scroll logic: scroll so cursor_doc_line is last visible
    dv.presentation
        .display
        .viewport
        .scroll_to_doc_line(cursor_doc_line.saturating_sub(visible_rows.saturating_sub(1)));
    let range = dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count());
    assert!(
        range.start <= cursor_doc_line && range.end > cursor_doc_line,
        "cursor doc line {cursor_doc_line} should be visible in range {range:?}"
    );
}

#[test]
fn viewport_scroll_small_file_visual_line() {
    // File smaller than viewport — scroll should clamp to 0
    let lines: Vec<String> = (0..3).map(|i| format!("line {i}")).collect();
    let mut dv = DocumentView::new(lines, 10, 10.0);
    let cursor_doc_line = 2usize;
    let visible_rows = dv.presentation.display.viewport.visible_rows;
    dv.presentation.display.display_map.set_entries(
        (0..dv.line_count())
            .map(|_| crate::snap_tree::DisplayLineEntry::placeholder(0, 10, 0, 1))
            .collect(),
    );

    dv.presentation
        .display
        .viewport
        .scroll_to_doc_line(cursor_doc_line.saturating_sub(visible_rows.saturating_sub(1)));
    // 2 - (10-1) = saturating to 0
    assert_eq!(dv.presentation.display.viewport.scroll_top, 0.0);
    let range = dv
        .presentation
        .display
        .viewport
        .visible_doc_line_range(&dv.presentation.display.display_map);
    assert!(range.start <= cursor_doc_line && range.end > cursor_doc_line);
}

// ── wrap 长行下半段光标可见性 ─────────────────────────────────

#[test]
fn ensure_cursor_visible_with_wrap_index() {
    // 构造一个超长行，让它 wrap 到多行
    // 每行 100 字符，viewport 宽度约 80 字符 → 每行 wrap 成 2 行
    let long_line = "a".repeat(300); // 300 字符，wrap 成约 4 行
    let lines: Vec<String> = vec![long_line.clone(), "short line".to_string()];
    let mut dv = DocumentView::new(lines, 10, 10.0);

    // 设置 dv.presentation.display.display_map 模拟 wrap 结果
    dv.presentation.display.display_map.set_entries(
        (0..2).map(|_| crate::snap_tree::DisplayLineEntry::placeholder(0, 10, 0, 1)).collect(),
    );
    let mut e = DisplayLineEntry::placeholder(0, 10, 0, 1);
    e.visual_line_count = 4;
    dv.presentation.display.display_map.update_entry_in_place(0, e);
    dv.presentation.display.display_map.rebuild_tree(); // 第 0 行 wrap 成 4 行
    let mut e = DisplayLineEntry::placeholder(0, 10, 0, 1);
    e.visual_line_count = 1;
    dv.presentation.display.display_map.update_entry_in_place(1, e);
    dv.presentation.display.display_map.rebuild_tree(); // 第 1 行是 1 行

    // 光标在第 0 行末尾（wrap 的下半段）
    let offset = long_line.len() - 1;
    dv.cursor_move_to_offset(offset);

    // Autoscroll using WrapIndex (doc-level precise)
    let cursor_doc_line = dv.cursor_line();
    let cursor_display = dv.presentation.display.display_map.doc_to_display(cursor_doc_line);
    let first_vl = dv.presentation.display.viewport.first_visible_row().as_usize();
    let visible_rows = dv.presentation.display.viewport.visible_rows;
    let last_vl = first_vl + visible_rows;

    if cursor_display < first_vl {
        dv.presentation
            .display
            .viewport
            .scroll_to_doc_line_wrap(cursor_doc_line, &dv.presentation.display.display_map);
    } else if cursor_display >= last_vl {
        let target_line = cursor_doc_line.saturating_sub(visible_rows.saturating_sub(1));
        dv.presentation
            .display
            .viewport
            .scroll_to_doc_line_wrap(target_line, &dv.presentation.display.display_map);
    }

    // 验证光标所在的 visual 行在 viewport 内
    let cursor_display = dv.presentation.display.display_map.doc_to_display(cursor_doc_line);
    let first_vl = dv.presentation.display.viewport.first_visible_row().as_usize();
    let visible_rows = dv.presentation.display.viewport.visible_rows;
    let last_vl = first_vl + visible_rows;

    assert!(
        cursor_display >= first_vl && cursor_display < last_vl,
        "cursor display row {cursor_display} should be in viewport [{first_vl}, {last_vl})"
    );
}

#[test]
fn ensure_cursor_visible_fallback_without_wrap_index() {
    // 测试近似路径（无 wrap_index）
    let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
    let mut dv = DocumentView::new(lines, 10, 10.0);

    // 光标在第 50 行
    let offset = dv.line_byte_offset(50).unwrap();
    dv.cursor_move_to_offset(offset);

    // Autoscroll using approximate path
    let line = dv.cursor_line();
    let range = dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count());
    if line < range.start {
        dv.presentation.display.viewport.scroll_to_doc_line(line);
    } else if line >= range.end {
        let target = line.saturating_sub(dv.presentation.display.viewport.visible_rows - 1);
        dv.presentation.display.viewport.scroll_to_doc_line(target);
    }

    // 验证光标在 viewport 内
    let range = dv.presentation.display.viewport.visible_doc_line_range_approx(dv.line_count());
    assert!(range.start <= 50 && range.end > 50, "line 50 should be visible in range {range:?}");
}

// ── visible_*_wrap 方法测试 ──────────────────────────────────────

#[cfg(test)]
mod visible_wrap_tests {
    use super::*;

    #[test]
    fn visible_line_count_wrap_uses_precise_range() {
        // 10 行文档，viewport 显示 5 行
        let lines: Vec<String> = (0..10).map(|i| format!("line {i}")).collect();
        let mut dv = DocumentView::new(lines, 5, 10.0);
        // use dv.presentation.display.display_map instead of separate wi
        dv.presentation.display.display_map.set_entries(
            (0..10).map(|_| crate::snap_tree::DisplayLineEntry::placeholder(0, 10, 0, 1)).collect(),
        );

        // 无 wrap 时，visible_line_count_wrap 应等于 min(visible_rows, total_lines)
        assert_eq!(dv.visible_line_count_wrap_with_line_height(TEST_LINE_HEIGHT), 5);
    }

    #[test]
    fn visible_line_wrap_returns_correct_line() {
        let lines: Vec<String> = (0..10).map(|i| format!("line {i}")).collect();
        let mut dv = DocumentView::new(lines, 5, 10.0);
        // use dv.presentation.display.display_map instead of separate wi
        dv.presentation.display.display_map.set_entries(
            (0..10).map(|_| crate::snap_tree::DisplayLineEntry::placeholder(0, 10, 0, 1)).collect(),
        );

        // vis_idx=0 应返回第 0 行
        let line0 = dv.visible_line_wrap_with_line_height(0, TEST_LINE_HEIGHT).unwrap();
        assert_eq!(&line0[..], b"line 0");

        // vis_idx=4 应返回第 4 行
        let line4 = dv.visible_line_wrap_with_line_height(4, TEST_LINE_HEIGHT).unwrap();
        assert_eq!(&line4[..], b"line 4");

        // vis_idx=5 超出可见范围
        assert!(dv.visible_line_wrap_with_line_height(5, TEST_LINE_HEIGHT).is_none());
    }

    #[test]
    fn visible_line_key_wrap_returns_correct_key() {
        let lines: Vec<String> = (0..10).map(|i| format!("line {i}")).collect();
        let mut dv = DocumentView::new(lines, 5, 10.0);
        // use dv.presentation.display.display_map instead of separate wi
        dv.presentation.display.display_map.set_entries(
            (0..10).map(|_| crate::snap_tree::DisplayLineEntry::placeholder(0, 10, 0, 1)).collect(),
        );

        // vis_idx=0 应返回第 0 行的 offset 和 length
        let (offset, length) =
            dv.visible_line_key_wrap_with_line_height(0, TEST_LINE_HEIGHT).unwrap();
        assert_eq!(offset, 0);
        assert!(length > 0);

        // vis_idx=5 超出可见范围
        assert!(dv.visible_line_key_wrap_with_line_height(5, TEST_LINE_HEIGHT).is_none());
    }

    #[test]
    fn visible_line_wrap_with_scroll() {
        let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
        let mut dv = DocumentView::new(lines.clone(), 10, 10.0);
        dv.presentation.display.display_map.set_entries(
            (0..100)
                .map(|_| crate::snap_tree::DisplayLineEntry::placeholder(0, 10, 0, 1))
                .collect(),
        );

        // 滚动到第 50 行（Stage 5: 用 anchor 替代直接写 scroll_top）
        dv.presentation.display.viewport.scroll_anchor = ui::viewport::ScrollAnchor::new(50, 0.0);
        dv.presentation.display.viewport.derive_scroll_top(
            &dv.presentation.display.display_map,
            ui::settings::Settings::new().line_height,
        );

        // vis_idx=0 应返回第 50 行
        let line = dv.visible_line_wrap_with_line_height(0, TEST_LINE_HEIGHT).unwrap();
        assert_eq!(&line[..], b"line 50");

        // vis_idx=9 应返回第 59 行
        let line = dv.visible_line_wrap_with_line_height(9, TEST_LINE_HEIGHT).unwrap();
        assert_eq!(&line[..], b"line 59");
    }

    #[test]
    fn visible_line_count_wrap_with_wrapping() {
        // 10 行文档，每行 wrap 成 2 行 → 20 display rows
        let lines: Vec<String> = (0..10).map(|i| format!("line {i}")).collect();
        let mut dv = DocumentView::new(lines, 15, 10.0);
        // use dv.presentation.display.display_map instead of separate wi
        dv.presentation.display.display_map.set_entries(
            (0..10).map(|_| crate::snap_tree::DisplayLineEntry::placeholder(0, 10, 0, 1)).collect(),
        );
        for i in 0..10 {
            let mut e = DisplayLineEntry::placeholder(0, 10, 0, 1);
            e.visual_line_count = 2;
            dv.presentation.display.display_map.update_entry_in_place(i, e);
            dv.presentation.display.display_map.rebuild_tree();
        }

        // viewport 显示 15 行，但只有 10 doc lines (20 display rows)
        // visible_line_count_wrap 按 doc line 范围计算
        let count = dv.visible_line_count_wrap_with_line_height(TEST_LINE_HEIGHT);
        // 应该返回可见范围内的 doc line 数
        assert!(count > 0);
        assert!(count <= 10);
    }
}
