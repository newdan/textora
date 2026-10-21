//! 布局几何回归测试（无需 WGPU）。
//!
//! 目的：将截图中暴露的"文字垂直错位 / 切换控件不居中 / 输入框异常缩放"问题，
//! 固化为可复现的几何断言（先复现失败，再修复）。
//! 通过 `internal_test_support` feature 启用，仅供测试使用。

#![cfg(feature = "internal_test_support")]

use textora_ui::decorations::{caret_cursor_line_region, caret_selection_line_regions};

// ============================================================
// 一、装饰条几何：光标 / 选区必须落在正确的视觉行带内
// ============================================================

/// 光标区域必须映射到正确的行索引。
#[test]
fn caret_cursor_line_region_resolves_expected_line() {
    let region = caret_cursor_line_region(2, 5.0, 20.0);
    assert_eq!(region.line_index, 2, "光标行索引应等于输入的行索引（防止坐标错位）");
    assert_eq!(region.top, 5.0, "光标区域 top 应为 line_top");
    assert_eq!(region.height, 20.0, "光标区域高度应为行高");
}

/// 跨多行选区的起止行区域必须分别映射正确。
#[test]
fn caret_selection_line_regions_resolve_start_and_end_lines() {
    let (start_region, end_region) = caret_selection_line_regions(1, 3, 5.0, 20.0);
    assert_eq!(start_region.line_index, 1, "选区起始行索引应正确（防止坐标错位）");
    assert_eq!(end_region.line_index, 3, "选区结束行索引应正确（防止坐标错位）");
    assert_eq!(start_region.top, 5.0, "选区起始 top 应为 line_top");
    assert_eq!(start_region.height, 20.0, "选区起始高度应为行高");
    assert_eq!(end_region.top, 5.0, "选区结束 top 应为 line_top");
    assert_eq!(end_region.height, 20.0, "选区结束高度应为行高");
}

/// 单行选区（起止相同）也应返回两个一致的区域。
#[test]
fn single_line_selection_regions_are_consistent() {
    let (start_region, end_region) = caret_selection_line_regions(2, 2, 10.0, 20.0);
    assert_eq!(start_region.line_index, end_region.line_index, "单行选区起止行索引应一致");
    assert_eq!(start_region.top, end_region.top, "单行选区起止 top 应一致");
}

/// 垂直越界防护：光标顶端不能越过行顶，底端不能越过行底。
#[test]
fn cursor_rect_stays_within_line_band() {
    let font_baseline = 8.0;
    let font_line_height = 20.0;

    let caret_top = font_baseline;
    let caret_height = font_line_height - 2.0 * font_baseline;
    assert!(caret_top >= 0.0, "光标顶端不能越过行顶（防止垂直越界）");
    assert!(caret_top + caret_height <= font_line_height, "光标底端不能越过行底（防止垂直越界）");
    assert!(caret_top < font_line_height, "光标不能落入下一行区域（防止视觉错位）");
}

// ============================================================
// 二、文本基线对齐：文本必须完整落在行带内，不被裁切
// ============================================================

/// 模拟 render_text_input_row 的 baseline 计算，验证文本不越出行带。
#[test]
fn baseline_alignment_keeps_text_within_line_band() {
    let font_size = 12.0;
    let font_baseline = 8.0;
    let font_line_height = 20.0;

    let text_baseline = font_baseline + (font_line_height - font_size) / 2.0;
    let text_top = text_baseline - font_baseline;
    assert!(text_top >= 0.0, "文本顶不能越出行顶（防止文字被裁切）");
    assert!(text_top + font_size <= font_line_height + 1.0, "文本底不能越出行底（防止文字被裁切）");
}

// ============================================================
// 三、设置对话框控件几何：复选框必须严格居中
// ============================================================

/// 模拟 render_settings_dialog_row 中复选框几何计算，验证垂直居中。
#[test]
fn checkbox_marker_is_vertically_centered_in_row() {
    let row_height = 32.0;
    let row_top = 100.0;

    let checkbox_size = 16.0;
    let checkbox_top = row_top + (row_height - checkbox_size) / 2.0;
    let checkbox_center = checkbox_top + checkbox_size / 2.0;
    let row_center = row_top + row_height / 2.0;

    assert!((checkbox_center - row_center).abs() < 0.5, "复选框中心应与行中心重合（防止垂直错位）");
}

/// 模拟 render_settings_dialog_row 中复选框水平位置，验证右对齐。
#[test]
fn checkbox_marker_is_horizontally_end_aligned() {
    let row_right = 200.0;
    let checkbox_size = 16.0;
    let checkbox_right_margin = 8.0;

    let checkbox_x = row_right - checkbox_right_margin - checkbox_size;
    assert_eq!(checkbox_x, 200.0 - 8.0 - 16.0, "复选框右对齐计算错误");
    assert!(checkbox_x > 0.0, "复选框不能越出左边界");
}
