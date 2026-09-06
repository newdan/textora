//! Markdown 感知的编辑增强（Enter / Backspace / Delete / InsertText）。
//!
//! 方案 2026-07-06 阶段 2b/2c：把原本散落在 `view.rs` 里的
//! - `classify_enter_context` 分类器
//! - `augment_enter` / `augment_backspace` / `augment_insert_text` 入口
//! - 7 个 `*_augmentation` 自由函数
//! 收敛到本模块，并把三条通用产物（source_newline / block_break / marker_break）
//! 抽成 `emit_*` 原语，让所有分支共享同一份不变量断言。
//!
//! 不变量（对每一次成功返回的 [`ui::plugin::EditAugmentation`]）：
//! 1. `replace_range.end <= source.len()`
//! 2. `cursor_byte_after` 位于最终字符串的合法字节位置
//! 3. `insert_text` 与 `replace_range` 至少有一个存在
//! 4. `emit_*` 三原语内部用 `debug_assert!` 强制以上约束

#[path = "editable_paragraph_edit.rs"]
mod editable_paragraph_edit;
#[path = "editable_paragraph_navigation.rs"]
mod editable_paragraph_navigation;

use crate::builder::ListBullet;
#[cfg(test)]
use std::cell::Cell;
use ui::plugin::{AugmentKind, EditAugmentation};
use unicode_segmentation::UnicodeSegmentation;

const LF_SEQUENCE: &str = "\n";
const CRLF_SEQUENCE: &str = "\r\n";
const BLOCK_BOUNDARY_NEWLINE_COUNT: usize = 2;
const MAX_LEADING_BLOCK_INDENT: usize = 3;
/// CommonMark 代码围栏(``` 或 ~~~)的最少字符数。
const MIN_FENCE_MARKER_LENGTH: usize = 3;
/// CommonMark 空格形式硬换行所需的最少行尾空格数。
const HARD_BREAK_MIN_SPACES: usize = 2;
const TASK_LIST_CONTENT_SEPARATOR: char = ' ';
const MARKDOWN_TAB_STOP_WIDTH: usize = 4;

#[cfg(test)]
thread_local! {
    static CLASSIFY_PARSE_COUNT: Cell<usize> = const { Cell::new(0) };
}

// ─── 入口 ──────────────────────────────────────────────────────────────────

/// 根据当前光标位置和键类型，计算一个 markdown 感知的编辑增强。
///
/// 返回 `None` 表示当前上下文没有特殊行为，调用方应回落到普通 Enter/Backspace/InsertText。
pub fn augment_edit(
    source: &str,
    current_byte: usize,
    kind: AugmentKind,
) -> Option<EditAugmentation> {
    match kind {
        AugmentKind::Enter => augment_enter(source, current_byte),
        AugmentKind::LineBreak => Some(augment_line_break(source, current_byte)),
        AugmentKind::Backspace => augment_backspace(source, current_byte),
        AugmentKind::Tab => None,
        AugmentKind::InsertText(ref text) => augment_insert_text(source, current_byte, text),
    }
}

fn augment_line_break(source: &str, current_byte: usize) -> EditAugmentation {
    let split_at = reversible_split_point(source, current_byte);
    let context = classify_enter_context(source, current_byte);
    let continuation_prefix = match &context {
        EnterContext::ListItem { content_prefix, .. } => content_prefix.clone(),
        EnterContext::BlockQuoteLine { continuation_prefix, .. }
        | EnterContext::IndentedCodeBlock { continuation_prefix } => continuation_prefix.clone(),
        EnterContext::LiteralBlockLine { continuation_prefix } => {
            return emit_literal_source_newline(source, current_byte, continuation_prefix);
        }
        EnterContext::Heading { .. }
        | EnterContext::SetextHeading { .. }
        | EnterContext::TableCell { .. } => {
            return emit_inline_html_break(source, current_byte);
        }
        EnterContext::CodeBlock
        | EnterContext::CodeBlockFenceLine { .. }
        | EnterContext::EmptyBlockSeparatorLine
        | EnterContext::Other => {
            return emit_source_newline(source, current_byte);
        }
        EnterContext::TopLevelParagraphEnd
        | EnterContext::ParagraphInterior
        | EnterContext::ParagraphStart { .. } => String::new(),
    };
    let newline = preferred_newline_sequence(source, split_at);
    let insertion = format!("\\{newline}{continuation_prefix}");
    let augmentation = EditAugmentation {
        cursor_byte_after: split_at + insertion.len(),
        replace_range: Some(split_at..split_at),
        insert_text: Some(insertion),
    };
    preserve_inline_elements_at_split(source, split_at, augmentation)
}

fn emit_inline_html_break(source: &str, current_byte: usize) -> EditAugmentation {
    const INLINE_HTML_BREAK: &str = "<br>";
    let augmentation = EditAugmentation {
        cursor_byte_after: current_byte + INLINE_HTML_BREAK.len(),
        replace_range: Some(current_byte..current_byte),
        insert_text: Some(INLINE_HTML_BREAK.to_owned()),
    };
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

fn augment_enter(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    if let Some(augmentation) = editable_paragraph_navigation::enter(source, current_byte) {
        return Some(augmentation);
    }
    let context = classify_enter_context(source, current_byte);
    enter_context_augmentation(source, current_byte, context)
}

fn augment_backspace(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    if let Some(aug) = editable_paragraph_navigation::backspace_boundary(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_remove_inline_html_break(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_join_reopened_inline_elements(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_empty_source_line(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_at_atx_heading_marker_start(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_join_hard_break_line(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_paragraph_boundary(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_last_isolated_paragraph_grapheme(source, current_byte) {
        return Some(aug);
    }
    let range = get_marker_delete_range(source, current_byte)?;
    Some(EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(range.clone()),
        cursor_byte_after: range.start,
    })
}

pub(crate) fn augment_delete_forward(
    source: &str,
    current_byte: usize,
) -> Option<EditAugmentation> {
    editable_paragraph_navigation::delete_forward(source, current_byte)
        .or_else(|| delete_forward_remove_inline_html_break(source, current_byte))
        .or_else(|| delete_forward_join_reopened_inline_elements(source, current_byte))
        .or_else(|| {
            let grapheme = source.get(current_byte..)?.graphemes(true).next()?;
            editable_paragraph_edit::erase_last_grapheme(source, current_byte + grapheme.len())
        })
        .or_else(|| delete_forward_block_boundary(source, current_byte))
}

pub(crate) fn augment_erase_range(
    source: &str,
    erased: std::ops::Range<usize>,
) -> Option<EditAugmentation> {
    editable_paragraph_edit::erase_range(source, erased)
}

fn delete_forward_remove_inline_html_break(
    source: &str,
    current_byte: usize,
) -> Option<EditAugmentation> {
    const INLINE_HTML_BREAKS: [&str; 3] = ["<br>", "<br/>", "<br />"];
    let suffix = source.get(current_byte..)?;
    let html_break =
        INLINE_HTML_BREAKS.into_iter().find(|html_break| suffix.starts_with(html_break))?;
    let replace_end = current_byte + html_break.len();
    let augmentation = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(current_byte..replace_end),
        cursor_byte_after: current_byte,
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

fn delete_forward_join_reopened_inline_elements(
    source: &str,
    current_byte: usize,
) -> Option<EditAugmentation> {
    let frames = inline_wrapper_frames(source);
    let mut previous_frames = frames
        .iter()
        .filter(|frame| {
            frame.content_end == current_byte
                && frame.source_range.start < current_byte
                && frame.source_range.end > current_byte
        })
        .collect::<Vec<_>>();
    previous_frames.sort_by_key(|frame| frame.source_range.start);
    let previous_outer = previous_frames.first()?;
    let mut boundary_start = previous_outer.source_range.end;
    let separator = if source.as_bytes().get(boundary_start) == Some(&b' ') {
        boundary_start += 1;
        " "
    } else {
        ""
    };

    let next_line_start =
        if let Some(hard_break) = hard_break_boundary_after(source, boundary_start) {
            hard_break.end
        } else {
            let mut probe = boundary_start;
            let mut newline_count = 0;
            while let Some(newline_width) = newline_sequence_width_at(source, probe) {
                probe += newline_width;
                newline_count += 1;
            }
            if newline_count == 0 {
                return None;
            }
            probe
        };
    let (_, _, next_line_end) = locate_source_line_bounds(source, next_line_start)?;
    let opening_start = container_content_start_on_line(source, next_line_start, next_line_end);
    let mut current_frames = frames
        .iter()
        .filter(|frame| {
            frame.source_range.start >= opening_start
                && frame.source_range.start < next_line_end
                && frame.source_range.start < frame.content_start
                && frame.content_start > opening_start
                && frame.content_start <= next_line_end
        })
        .collect::<Vec<_>>();
    current_frames.sort_by_key(|frame| frame.source_range.start);
    let current_outer = current_frames.first()?;
    if current_outer.source_range.start != opening_start
        || current_frames.iter().any(|frame| frame.content_start != current_outer.content_start)
        || !inline_paths_match(&previous_frames, &current_frames)
    {
        return None;
    }

    let augmentation = EditAugmentation {
        insert_text: Some(separator.to_owned()),
        replace_range: Some(current_byte..current_outer.content_start),
        cursor_byte_after: current_byte,
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

/// Delete(前向删除)的块边界护栏,与 Backspace 侧的
/// [`backspace_paragraph_boundary`] 对称。
///
/// 光标位于非空源码行内容末尾、紧邻换行 run 时:
/// - 下一物理行自成独立块(ATX/围栏/HR/列表 marker/setext 下划线/引用/表格行),
///   默认逐字符删除会把两行并线、破坏结构(如段落成 setext 标题)——
///   返回消费型空操作(不删任何字节);
/// - 当前块为围栏/缩进代码块时,只有下一行是闭合围栏才拦截,
///   普通代码行允许默认删除;
/// - 段落/标题末尾且下一行是普通段落文本:删除整个换行 run,合并两段
///   (与段首 Backspace 删整个边界对称)。
fn delete_forward_block_boundary(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    let boundary_end = text_block_source_end(source, current_byte)?;
    let consume_boundary = || {
        Some(EditAugmentation {
            insert_text: Some(String::new()),
            replace_range: None,
            cursor_byte_after: current_byte,
        })
    };
    if boundary_end == source.len() && current_byte != boundary_end {
        return consume_boundary();
    }
    let context = classify_enter_context(source, boundary_end);
    // ATX 标题尾部空白不产生段内硬换行。
    if !matches!(context, EnterContext::Heading { .. })
        && hard_break_marker_ending_at(source, boundary_end).is_some()
    {
        return None;
    }
    let first_newline_width = newline_sequence_width_at(source, boundary_end)?;
    let mut newline_run_end = boundary_end + first_newline_width;
    while let Some(newline_width) = newline_sequence_width_at(source, newline_run_end) {
        newline_run_end += newline_width;
    }
    if let Some(separator) = editable_paragraph_navigation::hidden_separator_range(
        source,
        boundary_end,
        editable_paragraph_navigation::Boundary::End,
    ) {
        newline_run_end = separator.end;
    }
    if newline_run_end >= source.len() {
        return None;
    }

    let container_path = delete_context_container_path(source, boundary_end, &context);
    match context {
        EnterContext::CodeBlock => {
            // 代码体内只有闭合围栏行需要保护;其余代码行交给默认删除。
            if newline_run_end == boundary_end + first_newline_width
                && line_closes_enclosing_fenced_block(source, boundary_end, newline_run_end)
            {
                return consume_boundary();
            }
            None
        }
        EnterContext::CodeBlockFenceLine { .. } => {
            // 围栏行与相邻行并线必然破坏围栏;仅在边界恰好一个换行序列时拦截,
            // 有空行兜底(≥2 个换行)时默认删除一个仍保持结构合法。
            if newline_run_end == boundary_end + first_newline_width {
                return consume_boundary();
            }
            None
        }
        EnterContext::TopLevelParagraphEnd
        | EnterContext::ParagraphInterior
        | EnterContext::ParagraphStart { .. }
        | EnterContext::Heading { .. } => {
            if line_starts_independent_block(source, newline_run_end, &container_path) {
                return consume_boundary();
            }
            let delete_start = if matches!(context, EnterContext::Heading { .. }) {
                atx_heading_merge_end(source, boundary_end)
            } else {
                boundary_end
            };
            let augmentation = EditAugmentation {
                insert_text: Some(String::new()),
                replace_range: Some(delete_start..newline_run_end),
                cursor_byte_after: current_byte.min(delete_start),
            };
            debug_assert_augmentation(&augmentation, source);
            Some(augmentation)
        }
        _ => {
            if line_starts_independent_block(source, newline_run_end, &container_path) {
                return consume_boundary();
            }
            None
        }
    }
}

/// Resolve a visible text endpoint to the end of its complete source line.
fn text_block_source_end(source: &str, current_byte: usize) -> Option<usize> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    let boundary_end = source_line_content_end(source, line_end);
    if line_start == current_byte {
        return None;
    }
    if current_byte != boundary_end {
        if source.get(current_byte..boundary_end)?.chars().next().is_some_and(char::is_alphanumeric)
        {
            return None;
        }
        let document =
            crate::builder::MarkdownDoc::build_structure(&crate::parser::parse_markdown(source));
        if !editable_paragraph_navigation::has_text_boundary(
            &document.blocks,
            current_byte,
            editable_paragraph_navigation::Boundary::End,
        ) {
            return None;
        }
    }
    Some(boundary_end)
}

/// Remove only the optional ATX closing sequence when serializing a joined heading.
fn atx_heading_merge_end(source: &str, line_end: usize) -> usize {
    let Some((line_start, _, _)) = locate_source_line_bounds(source, line_end) else {
        return line_end;
    };
    let marker_start = container_content_start_on_line(source, line_start, line_end);
    let line = &source[marker_start..line_end];
    let trimmed = line.trim_end_matches([' ', '\t']);
    let before_hashes = trimmed.trim_end_matches('#');
    if before_hashes.len() == trimmed.len() || !before_hashes.ends_with([' ', '\t']) {
        return line_end;
    }
    let opening_end = line.len() - line.trim_start_matches('#').len();
    let after_opening = &line[opening_end..];
    let content_start =
        opening_end + after_opening.len() - after_opening.trim_start_matches([' ', '\t']).len();
    let closing_start = before_hashes.trim_end_matches([' ', '\t']).len();
    marker_start + closing_start.max(content_start)
}

fn delete_context_container_path(
    source: &str,
    current_byte: usize,
    context: &EnterContext,
) -> ContainerPath {
    let prefix = match context {
        EnterContext::Heading { continuation_prefix, .. }
        | EnterContext::SetextHeading { continuation_prefix, .. }
        | EnterContext::BlockQuoteLine { continuation_prefix, .. }
        | EnterContext::IndentedCodeBlock { continuation_prefix }
        | EnterContext::LiteralBlockLine { continuation_prefix } => continuation_prefix,
        EnterContext::ListItem { content_prefix, .. } => content_prefix,
        _ => "",
    };
    let base_path = container_path(prefix);
    let Some((line_start, _, _)) = locate_source_line_bounds(source, current_byte) else {
        return base_path;
    };
    source_container_path(source, line_start, current_byte, &base_path)
}

/// Delete 侧判定:下一物理行是否自成独立块,默认前向删除并线后会破坏结构。
/// 在 [`line_starts_new_sibling_block`] 基础上补充 setext 下划线、
/// 引用行(`>`)与表格行(`|`);后两者可被前一块合法中断,因此不并入
/// `line_starts_new_sibling_block`,避免影响 Backspace 侧既有行为。
fn line_starts_independent_block(
    source: &str,
    line_start: usize,
    container_path: &ContainerPath,
) -> bool {
    let Some(marker_line) = block_marker_line(source, line_start, container_path)
        .or_else(|| block_marker_line(source, line_start, &ContainerPath::default()))
    else {
        return false;
    };
    line_starts_new_sibling_marker(source, &marker_line)
        || matches!(source.as_bytes().get(marker_line.marker_start), Some(b'>' | b'|'))
        || content_is_setext_underline(marker_line.content)
}

struct BlockMarkerLine<'a> {
    marker_start: usize,
    content: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LineColumnPosition {
    byte_offset: usize,
    /// 候选物理行按 CommonMark tab stop 展开后的绝对列。
    absolute_column: usize,
    /// 当前容器路径要求到达的绝对列；两列之差是跨界 Tab 留给叶块的虚拟缩进。
    container_target_column: usize,
}

impl LineColumnPosition {
    fn at_line_start(line_start: usize) -> Self {
        Self { byte_offset: line_start, absolute_column: 0, container_target_column: 0 }
    }

    fn at_byte(source: &str, line_start: usize, byte_offset: usize) -> Option<Self> {
        let prefix = source.get(line_start..byte_offset)?;
        let absolute_column = markdown_column_after(prefix, 0);
        Some(Self { byte_offset, absolute_column, container_target_column: absolute_column })
    }

    fn leaf_indent_columns(self) -> Option<usize> {
        self.absolute_column.checked_sub(self.container_target_column)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ContainerPath(Vec<ContainerSegment>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContainerSegment {
    BlockQuote,
    IndentColumns(usize),
}

/// 先精确消费容器必需续行前缀，再定位至多 3 空格缩进的块 marker。
fn block_marker_line<'a>(
    source: &'a str,
    line_start: usize,
    container_path: &ContainerPath,
) -> Option<BlockMarkerLine<'a>> {
    let (actual_line_start, _, line_end) = locate_source_line_bounds(source, line_start)?;
    if actual_line_start != line_start {
        return None;
    }
    let bytes = source.as_bytes();
    let content_end = source_line_content_end(source, line_end);
    let container_position =
        matching_container_position(source, line_start, content_end, container_path)?;
    let marker_position =
        marker_position_after_optional_indent(source, content_end, container_position)?;
    let marker_start = marker_position.byte_offset;
    let mut marker_end = content_end;
    while marker_end > marker_start && matches!(bytes[marker_end - 1], b' ' | b'\t') {
        marker_end -= 1;
    }
    Some(BlockMarkerLine { marker_start, content: source.get(marker_start..marker_end)? })
}

fn content_is_setext_underline(content: &str) -> bool {
    let Some(marker) = content.as_bytes().first().copied() else {
        return false;
    };
    matches!(marker, b'=' | b'-') && content.bytes().all(|byte| byte == marker)
}

fn content_is_thematic_break(content: &str) -> bool {
    let mut marker = None;
    let mut marker_count = 0;
    for byte in content.bytes() {
        if matches!(byte, b' ' | b'\t') {
            continue;
        }
        if !matches!(byte, b'*' | b'-' | b'_') || marker.is_some_and(|marker| marker != byte) {
            return false;
        }
        marker = Some(byte);
        marker_count += 1;
    }
    marker_count >= 3
}

fn backspace_remove_inline_html_break(
    source: &str,
    current_byte: usize,
) -> Option<EditAugmentation> {
    const INLINE_HTML_BREAKS: [&str; 3] = ["<br>", "<br/>", "<br />"];
    let prefix = source.get(..current_byte)?;
    let html_break =
        INLINE_HTML_BREAKS.into_iter().find(|html_break| prefix.ends_with(html_break))?;
    let replace_start = current_byte - html_break.len();
    let augmentation = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(replace_start..current_byte),
        cursor_byte_after: replace_start,
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

fn backspace_join_reopened_inline_elements(
    source: &str,
    current_byte: usize,
) -> Option<EditAugmentation> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    if current_byte == line_start {
        return None;
    }

    let frames = inline_wrapper_frames(source);
    let mut current_frames = frames
        .iter()
        .filter(|frame| {
            frame.content_start == current_byte
                && frame.source_range.start >= line_start
                && frame.source_range.start < current_byte
        })
        .collect::<Vec<_>>();
    current_frames.sort_by_key(|frame| frame.source_range.start);
    let current_outer = current_frames.first()?;
    let opening_start = current_outer.source_range.start;
    if container_content_start_on_line(source, line_start, line_end) != opening_start {
        return None;
    }

    let mut previous_wrapper_end = line_start;
    let mut newline_count = 0;
    while let Some(newline_width) = newline_sequence_width_before(source, previous_wrapper_end) {
        previous_wrapper_end -= newline_width;
        newline_count += 1;
    }
    if newline_count == 0 {
        return None;
    }
    if let Some(marker) = hard_break_marker_ending_at(source, previous_wrapper_end) {
        previous_wrapper_end = marker.start;
    }

    let separator = if source.as_bytes().get(previous_wrapper_end.wrapping_sub(1)) == Some(&b' ') {
        previous_wrapper_end -= 1;
        " "
    } else {
        ""
    };

    let previous_outer = frames
        .iter()
        .filter(|frame| frame.source_range.end == previous_wrapper_end)
        .min_by_key(|frame| frame.source_range.start)?;
    let closing_start = previous_outer.content_end;
    let mut previous_frames = frames
        .iter()
        .filter(|frame| {
            frame.content_end == closing_start
                && frame.source_range.start >= previous_outer.source_range.start
                && frame.source_range.end <= previous_outer.source_range.end
        })
        .collect::<Vec<_>>();
    previous_frames.sort_by_key(|frame| frame.source_range.start);

    if !inline_paths_match(&previous_frames, &current_frames) {
        return None;
    }

    let augmentation = EditAugmentation {
        insert_text: Some(separator.to_owned()),
        replace_range: Some(closing_start..current_byte),
        cursor_byte_after: closing_start + separator.len(),
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

fn inline_paths_match(
    previous_frames: &[&InlineWrapperFrame<'_>],
    current_frames: &[&InlineWrapperFrame<'_>],
) -> bool {
    previous_frames.len() == current_frames.len()
        && previous_frames.iter().zip(current_frames).all(|(previous, current)| {
            if previous.kind != current.kind {
                return false;
            }
            if matches!(previous.kind, InlineFrameKind::Link | InlineFrameKind::Code) {
                return previous.opening == current.opening && previous.closing == current.closing;
            }
            true
        })
}

fn backspace_at_atx_heading_marker_start(
    source: &str,
    current_byte: usize,
) -> Option<EditAugmentation> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    let line_content_end = source_line_content_end(source, line_end);
    let line = source.as_bytes().get(line_start..line_content_end)?;
    let leading_spaces =
        line.iter().take(MAX_LEADING_BLOCK_INDENT).take_while(|&&byte| byte == b' ').count();
    let marker_start = line_start + leading_spaces;
    if current_byte != marker_start {
        return None;
    }

    let marker_width = line[leading_spaces..].iter().take_while(|&&byte| byte == b'#').count();
    if !(1..=6).contains(&marker_width)
        || !matches!(line.get(leading_spaces + marker_width), None | Some(b' ' | b'\t'))
    {
        return None;
    }

    let augmentation = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: None,
        cursor_byte_after: current_byte,
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

fn backspace_last_isolated_paragraph_grapheme(
    source: &str,
    current_byte: usize,
) -> Option<EditAugmentation> {
    editable_paragraph_edit::erase_last_grapheme(source, current_byte)
}

fn newline_sequence_width_before(source: &str, byte: usize) -> Option<usize> {
    let prefix = source.as_bytes().get(..byte)?;
    if prefix.ends_with(b"\r\n") {
        return Some(2);
    }
    prefix.ends_with(b"\n").then_some(1)
}

fn newline_sequence_width_at(source: &str, byte: usize) -> Option<usize> {
    let suffix = source.as_bytes().get(byte..)?;
    if suffix.starts_with(b"\r\n") {
        return Some(2);
    }
    suffix.starts_with(b"\n").then_some(1)
}

/// 返回紧邻 `content_end` 结束的 Markdown 硬换行标记范围。
///
/// 空格形式消耗全部行尾空格。反斜杠形式按奇偶性解释，奇数个连续反斜杠中的
/// 最后一个是硬换行标记，前面的偶数个是转义后的正文。
fn hard_break_marker_ending_at(source: &str, content_end: usize) -> Option<std::ops::Range<usize>> {
    let prefix = source.as_bytes().get(..content_end)?;
    let trailing_backslash_count =
        prefix.iter().rev().take_while(|source_byte| **source_byte == b'\\').count();
    if !trailing_backslash_count.is_multiple_of(2) {
        return Some(content_end - 1..content_end);
    }

    let trailing_space_count =
        prefix.iter().rev().take_while(|source_byte| **source_byte == b' ').count();
    (trailing_space_count >= HARD_BREAK_MIN_SPACES)
        .then_some(content_end - trailing_space_count..content_end)
}

/// 光标位于硬换行源码标记之前时，返回应升级为块边界的完整源码范围。
///
/// 对奇数个连续反斜杠只替换最后一个标记及换行，保留前面的转义反斜杠。
fn hard_break_boundary_after(source: &str, current_byte: usize) -> Option<std::ops::Range<usize>> {
    let suffix = source.as_bytes().get(current_byte..)?;
    let trailing_backslash_count =
        suffix.iter().take_while(|source_byte| **source_byte == b'\\').count();
    if trailing_backslash_count > 0 {
        if trailing_backslash_count.is_multiple_of(2) {
            return None;
        }
        let marker_start = current_byte + trailing_backslash_count - 1;
        let newline_start = marker_start + 1;
        let newline_width = newline_sequence_width_at(source, newline_start)?;
        return Some(marker_start..newline_start + newline_width);
    }

    let trailing_space_count =
        suffix.iter().take_while(|source_byte| **source_byte == b' ').count();
    if trailing_space_count < HARD_BREAK_MIN_SPACES {
        return None;
    }
    let newline_start = current_byte + trailing_space_count;
    let newline_width = newline_sequence_width_at(source, newline_start)?;
    Some(current_byte..newline_start + newline_width)
}

fn preferred_newline_sequence(source: &str, current_byte: usize) -> &'static str {
    let bytes = source.as_bytes();
    let cursor = current_byte.min(bytes.len());
    let prefix = &bytes[..cursor];
    let suffix = &bytes[cursor..];

    if suffix.starts_with(CRLF_SEQUENCE.as_bytes())
        || prefix.ends_with(CRLF_SEQUENCE.as_bytes())
        || (prefix.ends_with(b"\r") && suffix.starts_with(b"\n"))
    {
        return CRLF_SEQUENCE;
    }
    if suffix.starts_with(LF_SEQUENCE.as_bytes()) || prefix.ends_with(LF_SEQUENCE.as_bytes()) {
        return LF_SEQUENCE;
    }

    let Some(first_line_feed) = bytes.iter().position(|source_byte| *source_byte == b'\n') else {
        return LF_SEQUENCE;
    };
    if first_line_feed > 0 && bytes[first_line_feed - 1] == b'\r' {
        CRLF_SEQUENCE
    } else {
        LF_SEQUENCE
    }
}

fn augment_insert_text(source: &str, current_byte: usize, text: &str) -> Option<EditAugmentation> {
    editable_paragraph_edit::insert_text(source, current_byte, text)
}

// ─── emit_* 原语 ──────────────────────────────────────────────────────────

/// 在当前位置插入单个 `\n`。用于块间空行 run 里"加空行"的语义（Typora 风格）。
fn emit_source_newline(source: &str, current_byte: usize) -> EditAugmentation {
    let insertion = String::from(preferred_newline_sequence(source, current_byte));
    let aug = EditAugmentation {
        cursor_byte_after: current_byte + insertion.len(),
        insert_text: Some(insertion),
        ..Default::default()
    };
    debug_assert_augmentation(&aug, source);
    aug
}

fn emit_literal_source_newline(
    source: &str,
    current_byte: usize,
    continuation_prefix: &str,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, current_byte);
    let insertion = format!("{newline}{continuation_prefix}");
    let augmentation = EditAugmentation {
        cursor_byte_after: current_byte + insertion.len(),
        replace_range: Some(current_byte..current_byte),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

/// Create one editable paragraph using the same separator roles as the layout tree.
fn emit_block_break(source: &str, current_byte: usize) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, current_byte);
    let insertion = newline.repeat(block_break_newline_count(source, current_byte));
    let augmentation = EditAugmentation {
        cursor_byte_after: current_byte + newline.len() * BLOCK_BOUNDARY_NEWLINE_COUNT,
        insert_text: Some(insertion),
        ..Default::default()
    };
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

fn block_break_newline_count(source: &str, current_byte: usize) -> usize {
    let Some(newline_width) = newline_sequence_width_at(source, current_byte) else {
        return BLOCK_BOUNDARY_NEWLINE_COUNT;
    };
    let next_line_start = current_byte + newline_width;
    if blank_source_line_width(source, next_line_start).is_none() {
        return BLOCK_BOUNDARY_NEWLINE_COUNT;
    }
    let document =
        crate::builder::MarkdownDoc::build_structure(&crate::parser::parse_markdown(source));
    let paragraphs = crate::builder::EditableParagraphMap::from_blocks(&document.blocks, source);
    let Some(run) = paragraphs.run_at_byte(next_line_start) else {
        return BLOCK_BOUNDARY_NEWLINE_COUNT;
    };
    let separator_becomes_hidden = run.has_preceding_block && run.hidden_separator_count == 0;
    1 + usize::from(separator_becomes_hidden)
}

fn emit_paragraph_before(
    source: &str,
    current_byte: usize,
    insert_at: usize,
    continuation_prefix: &str,
) -> EditAugmentation {
    let list_start = leading_heading_list_start(source, insert_at);
    let container_prefix;
    let (insert_at, continuation_prefix) = if let Some(marker_start) = list_start {
        container_prefix = canonical_container_prefix(source, marker_start);
        (marker_start, container_prefix.as_str())
    } else {
        (insert_at, continuation_prefix)
    };
    let insertion_unit =
        format!("{}{continuation_prefix}", preferred_newline_sequence(source, insert_at));
    let insertion = insertion_unit.repeat(preceding_paragraph_insertion_count(
        source,
        insert_at,
        &insertion_unit,
    ));
    let augmentation = EditAugmentation {
        cursor_byte_after: current_byte + insertion.len(),
        replace_range: Some(insert_at..insert_at),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

fn preceding_paragraph_insertion_count(source: &str, insert_at: usize, insertion: &str) -> usize {
    let mut prospective = source.to_owned();
    prospective.insert_str(insert_at, insertion);
    let document =
        crate::builder::MarkdownDoc::build_structure(&crate::parser::parse_markdown(&prospective));
    let paragraphs =
        crate::builder::EditableParagraphMap::from_blocks(&document.blocks, &prospective);
    let hidden = paragraphs.run_at_byte(insert_at).is_some_and(|run| {
        run.lines
            .iter()
            .position(|line| {
                line.source_range.start <= insert_at && insert_at <= line.source_range.end
            })
            .is_some_and(|index| index < run.hidden_separator_count)
    });
    1 + usize::from(hidden)
}

fn leading_heading_list_start(source: &str, heading_start: usize) -> Option<usize> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    struct LeadingListItem {
        marker_start: usize,
        has_content: bool,
    }
    let mut owners = Vec::<LeadingListItem>::new();
    for (event, range) in
        Parser::new_ext(source, crate::parser::markdown_options()).into_offset_iter()
    {
        if matches!(event, Event::Start(Tag::Heading { .. })) && range.start == heading_start {
            return owners.iter().find(|owner| !owner.has_content).map(|owner| owner.marker_start);
        }
        match event {
            Event::Start(Tag::Item) => {
                owners.push(LeadingListItem { marker_start: range.start, has_content: false })
            }
            Event::End(TagEnd::Item) => {
                owners.pop();
            }
            Event::Text(_)
            | Event::Code(_)
            | Event::Html(_)
            | Event::InlineHtml(_)
            | Event::Rule
            | Event::Start(
                Tag::CodeBlock(_) | Tag::Heading { .. } | Tag::Table(_) | Tag::Image { .. },
            ) => {
                for owner in &mut owners {
                    owner.has_content = true;
                }
            }
            _ => {}
        }
        if range.start > heading_start {
            return None;
        }
    }
    None
}

fn is_visible_content_start(source: &str, source_start: usize, cursor: usize) -> bool {
    cursor == source_start
        || inline_wrapper_frames(source)
            .iter()
            .any(|frame| frame.source_range.start == source_start && frame.content_start == cursor)
}

/// 用一个块边界替换现有硬换行源码，光标落在新块起点。
fn emit_block_break_replacing(source: &str, replaced: std::ops::Range<usize>) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, replaced.start);
    let insertion = newline.repeat(BLOCK_BOUNDARY_NEWLINE_COUNT);
    let aug = EditAugmentation {
        cursor_byte_after: replaced.start + insertion.len(),
        replace_range: Some(replaced),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&aug, source);
    aug
}

/// 与 [`emit_block_break`] 相同，但插入点由上下文指定而不是使用原光标。
fn emit_block_break_at(source: &str, insert_at: usize) -> EditAugmentation {
    let mut aug = emit_block_break(source, insert_at);
    aug.replace_range = Some(insert_at..insert_at);
    debug_assert_augmentation(&aug, source);
    aug
}

fn backspace_empty_source_line(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    editable_paragraph_navigation::backspace(source, current_byte)
}

fn contiguous_newline_count_before(source: &str, byte: usize) -> usize {
    let mut newline_count = 0;
    let mut run_start = byte.min(source.len());
    while let Some(newline_width) = newline_sequence_width_before(source, run_start) {
        run_start -= newline_width;
        newline_count += 1;
    }
    newline_count
}

fn backspace_paragraph_boundary(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    if current_byte != line_start
        || line_start == line_end
        || !matches!(
            classify_enter_context(source, current_byte),
            EnterContext::TopLevelParagraphEnd
                | EnterContext::ParagraphInterior
                | EnterContext::ParagraphStart { .. }
        )
    {
        return None;
    }

    let source_bytes = source.as_bytes();
    let mut boundary_start = current_byte;
    while boundary_start > 0 && source_bytes[boundary_start - 1] == b'\n' {
        boundary_start -= 1;
        if boundary_start > 0 && source_bytes[boundary_start - 1] == b'\r' {
            boundary_start -= 1;
        }
    }
    if let Some(separator) = editable_paragraph_navigation::hidden_separator_range(
        source,
        current_byte,
        editable_paragraph_navigation::Boundary::Start,
    ) {
        boundary_start = separator.start;
    }
    if boundary_start == current_byte {
        return None;
    }

    let context = classify_enter_context(source, boundary_start);
    match context {
        EnterContext::TopLevelParagraphEnd
        | EnterContext::ParagraphInterior
        | EnterContext::ParagraphStart { .. }
        | EnterContext::Heading { .. } => {
            let delete_start = if matches!(context, EnterContext::Heading { .. }) {
                atx_heading_merge_end(source, boundary_start)
            } else {
                hard_break_marker_ending_at(source, boundary_start)
                    .map_or(boundary_start, |marker| marker.start)
            };
            let aug = EditAugmentation {
                insert_text: Some(String::new()),
                replace_range: Some(delete_start..current_byte),
                cursor_byte_after: delete_start,
            };
            debug_assert_augmentation(&aug, source);
            Some(aug)
        }
        EnterContext::BlockQuoteLine { continuation_prefix, .. } => Some(
            merge_into_preceding_block(source, boundary_start, current_byte, &continuation_prefix),
        ),
        EnterContext::ListItem { content_prefix, .. } => {
            Some(merge_into_preceding_block(source, boundary_start, current_byte, &content_prefix))
        }
        _ => guard_unmergeable_leaf_boundary(source, boundary_start, current_byte),
    }
}

/// 段首退格并入引用块/列表项：删除块间空行分隔，为并入行补显式行首前缀
/// （引用的 `> ` marker，或列表延续行的内容列缩进），避免产生无 marker 的
/// 脆弱 lazy continuation 中间态。单条 augmentation 保证一次 undo 完整还原。
fn merge_into_preceding_block(
    source: &str,
    boundary_start: usize,
    current_byte: usize,
    line_prefix: &str,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, current_byte);
    let insert_text = format!("{newline}{line_prefix}");
    let cursor_byte_after = boundary_start + insert_text.len();
    let aug = EditAugmentation {
        insert_text: Some(insert_text),
        replace_range: Some(boundary_start..current_byte),
        cursor_byte_after,
    };
    debug_assert_augmentation(&aug, source);
    aug
}

/// 段首退格且前块为代码块/分隔线/setext 标题等不可安全合并的叶块时：
/// - 边界含空行（≥2 个换行序列）：默认计划只删一个换行，段落上移到叶块下一行，
///   结构保持合法，交回默认计划（返回 `None`）；
/// - 边界只剩单个换行：再删会把段落并到叶块所在行（` ```para `、`---para`、
///   `Title\n===para`），必须拦截。返回一个空操作 augmentation——
///   `insert_text` 为空、无 `replace_range`、光标不动——`view.rs` 的
///   `augmentation_edit_plan` 会把它映射为 `EditPlan::Consume`，
///   阻止默认的逐 grapheme 删除。
fn guard_unmergeable_leaf_boundary(
    source: &str,
    boundary_start: usize,
    current_byte: usize,
) -> Option<EditAugmentation> {
    debug_assert!(boundary_start < current_byte);
    if contiguous_newline_count_before(source, current_byte) > 1 {
        return None;
    }
    let aug = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: None,
        cursor_byte_after: current_byte,
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}

fn backspace_join_hard_break_line(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    let (line_start, _, _) = locate_source_line_bounds(source, current_byte)?;
    let continuation_prefix = source.get(line_start..current_byte)?;
    if !continuation_prefix_is_joinable(continuation_prefix)
        || line_starts_new_sibling_block(source, line_start)
    {
        return None;
    }

    let newline_width = newline_sequence_width_before(source, line_start)?;
    let previous_content_end = line_start - newline_width;
    let marker = hard_break_marker_ending_at(source, previous_content_end)?;
    if !continuation_prefix_matches_previous_container(source, line_start, continuation_prefix) {
        return Some(EditAugmentation {
            insert_text: Some(String::new()),
            replace_range: Some(current_byte..current_byte),
            cursor_byte_after: current_byte,
        });
    }
    let aug = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(marker.start..current_byte),
        cursor_byte_after: marker.start,
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}

fn continuation_prefix_matches_previous_container(
    source: &str,
    line_start: usize,
    continuation_prefix: &str,
) -> bool {
    if continuation_prefix.is_empty() {
        return true;
    }

    let Some(newline_width) = newline_sequence_width_before(source, line_start) else {
        return false;
    };
    let previous_line_end = line_start - newline_width;
    let previous_line_start = source[..previous_line_end]
        .bytes()
        .rposition(|source_byte| source_byte == b'\n')
        .map_or(0, |newline| newline + 1);
    let previous_content_start =
        container_content_start_on_line(source, previous_line_start, previous_line_end);
    canonical_container_prefix(source, previous_content_start) == continuation_prefix
}

fn container_content_start_on_line(source: &str, line_start: usize, line_end: usize) -> usize {
    let bytes = source.as_bytes();
    let mut probe = line_start;
    while probe < line_end && matches!(bytes.get(probe), Some(b' ' | b'\t')) {
        probe += 1;
    }

    loop {
        if let Some((_, content_start)) = parse_list_marker(source, probe)
            && content_start <= line_end
        {
            probe = content_start;
        } else if bytes.get(probe) == Some(&b'>') {
            probe += 1;
            if matches!(bytes.get(probe), Some(b' ' | b'\t')) {
                probe += 1;
            }
        } else {
            return probe;
        }
    }
}

fn continuation_prefix_is_joinable(prefix: &str) -> bool {
    prefix.bytes().all(|byte| matches!(byte, b' ' | b'\t' | b'>'))
}

fn line_starts_new_sibling_block(source: &str, line_start: usize) -> bool {
    block_marker_line(source, line_start, &ContainerPath::default())
        .is_some_and(|marker_line| line_starts_new_sibling_marker(source, &marker_line))
}

fn line_starts_new_sibling_marker(source: &str, marker_line: &BlockMarkerLine<'_>) -> bool {
    parse_list_marker(source, marker_line.marker_start).is_some()
        || heading_source_is_atx(source, marker_line.marker_start)
        || content_is_thematic_break(marker_line.content)
        || marker_line.content.starts_with("```")
        || marker_line.content.starts_with("~~~")
}

/// 用于 list / blockquote 的"续 marker"：插入 `"\n{indent}{marker}"`。
fn emit_marker_break(
    source: &str,
    current_byte: usize,
    indent: &str,
    marker: &str,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, current_byte);
    let insertion = format!("{newline}{indent}{marker}");
    let cursor_after = current_byte + insertion.len();
    let aug = EditAugmentation {
        insert_text: Some(insertion),
        cursor_byte_after: cursor_after,
        ..Default::default()
    };
    debug_assert_augmentation(&aug, source);
    aug
}

fn emit_marker_break_at(
    source: &str,
    current_byte: usize,
    indent: &str,
    marker: &str,
) -> EditAugmentation {
    let mut augmentation = emit_marker_break(source, current_byte, indent, marker);
    augmentation.replace_range = Some(current_byte..current_byte);
    augmentation
}

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

/// 将当前源码行替换为父容器前缀。空前缀表示退出顶层容器。
fn emit_replace_current_line(
    source: &str,
    current_byte: usize,
    parent_prefix: &str,
) -> Option<EditAugmentation> {
    let (start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    let content_end = source_line_content_end(source, line_end);
    let aug = EditAugmentation {
        insert_text: Some(parent_prefix.to_owned()),
        replace_range: Some(start..content_end),
        cursor_byte_after: start + parent_prefix.len(),
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}

fn debug_assert_augmentation(aug: &EditAugmentation, source: &str) {
    debug_assert!(
        aug.insert_text.is_some() || aug.replace_range.is_some(),
        "augmentation must produce at least one of insert_text / replace_range"
    );
    if let Some(range) = &aug.replace_range {
        debug_assert!(
            range.end <= source.len(),
            "replace_range.end {} exceeds source.len() {}",
            range.end,
            source.len()
        );
        debug_assert!(range.start <= range.end);
    }
}

// ─── 分类结果 → 具体 augmentation ────────────────────────────────────────

fn enter_context_augmentation(
    source: &str,
    current_byte: usize,
    context: EnterContext,
) -> Option<EditAugmentation> {
    match context {
        EnterContext::TopLevelParagraphEnd | EnterContext::ParagraphInterior => {
            Some(paragraph_enter_augmentation(source, current_byte))
        }
        EnterContext::ParagraphStart { insert_at } => {
            Some(emit_paragraph_before(source, current_byte, insert_at, ""))
        }
        EnterContext::Heading { level: _, position, continuation_prefix } => {
            heading_enter_augmentation(source, current_byte, position, &continuation_prefix)
        }
        EnterContext::SetextHeading { position, underline_end, continuation_prefix } => {
            match position {
                HeadingPosition::Start { insert_at } => Some(emit_paragraph_before(
                    source,
                    current_byte,
                    insert_at,
                    &continuation_prefix,
                )),
                HeadingPosition::Interior | HeadingPosition::End { .. } => Some(
                    setext_heading_enter_augmentation(source, underline_end, &continuation_prefix),
                ),
            }
        }
        EnterContext::ListItem {
            indent,
            continuation_marker,
            content_prefix,
            empty,
            at_end: _,
        } => list_item_enter_augmentation(
            source,
            current_byte,
            &indent,
            &continuation_marker,
            &content_prefix,
            empty,
        ),
        EnterContext::BlockQuoteLine { continuation_prefix, empty_parent_prefix, empty } => {
            blockquote_enter_augmentation(
                source,
                current_byte,
                &continuation_prefix,
                &empty_parent_prefix,
                empty,
            )
        }
        EnterContext::TableCell {
            next_cell_start,
            column_count,
            row_is_empty,
            is_header_row,
            row_line_end,
            container_prefix,
        } => {
            if let Some(next_cell_start) = next_cell_start {
                Some(EditAugmentation {
                    insert_text: Some(String::new()),
                    replace_range: None,
                    cursor_byte_after: next_cell_start,
                })
            } else if row_is_empty && !is_header_row {
                table_exit_empty_row_augmentation(source, row_line_end, &container_prefix)
            } else {
                Some(table_insert_row_augmentation(
                    source,
                    row_line_end,
                    column_count,
                    &container_prefix,
                ))
            }
        }
        EnterContext::EmptyBlockSeparatorLine => Some(emit_source_newline(source, current_byte)),
        EnterContext::IndentedCodeBlock { continuation_prefix } => {
            Some(indented_code_block_enter_augmentation(source, current_byte, &continuation_prefix))
        }
        EnterContext::LiteralBlockLine { continuation_prefix } => {
            Some(emit_literal_source_newline(source, current_byte, &continuation_prefix))
        }
        EnterContext::CodeBlockFenceLine { is_opening, line_content_end } => {
            Some(fence_line_enter_augmentation(source, line_content_end, is_opening))
        }
        EnterContext::CodeBlock | EnterContext::Other => None,
    }
}

fn paragraph_enter_augmentation(source: &str, current_byte: usize) -> EditAugmentation {
    if let Some(boundary) = hard_break_boundary_after(source, current_byte) {
        return emit_block_break_replacing(source, boundary);
    }

    // 段尾/段中：Typora 语义 = 切两半（`\n\n`）；跨已有 `\n` 时用
    // emit_block_break 保留光标下方的原源码行。
    if cursor_touches_source_newline(source, current_byte) {
        if newline_sequence_width_at(source, current_byte).is_some() {
            return emit_block_break(source, current_byte);
        }
        return emit_source_newline(source, current_byte);
    }
    let split_at = reversible_split_point(source, current_byte);
    let augmentation = if split_at == current_byte {
        emit_block_break(source, current_byte)
    } else {
        emit_block_break_at(source, split_at)
    };
    preserve_inline_elements_at_split(source, split_at, augmentation)
}

fn heading_enter_augmentation(
    source: &str,
    current_byte: usize,
    position: HeadingPosition,
    continuation_prefix: &str,
) -> Option<EditAugmentation> {
    if let HeadingPosition::Start { insert_at } = position {
        return Some(emit_paragraph_before(source, current_byte, insert_at, continuation_prefix));
    }
    if let HeadingPosition::End { insert_at } = position {
        if !continuation_prefix.is_empty() {
            return Some(emit_container_line_break(source, insert_at, continuation_prefix));
        }
        return Some(emit_block_break_at(source, insert_at));
    }

    // Heading 中间：在当前光标处分割标题，后半段成为普通段落。
    let split_at = reversible_split_point(source, current_byte);
    let insertion =
        format!("{}{}", preferred_newline_sequence(source, split_at), continuation_prefix);
    let aug = EditAugmentation {
        insert_text: Some(insertion.clone()),
        cursor_byte_after: split_at + insertion.len(),
        replace_range: Some(split_at..split_at),
    };
    debug_assert_augmentation(&aug, source);
    Some(preserve_inline_elements_at_split(source, split_at, aug))
}

fn emit_container_line_break(
    source: &str,
    current_byte: usize,
    continuation_prefix: &str,
) -> EditAugmentation {
    let insertion =
        format!("{}{}", preferred_newline_sequence(source, current_byte), continuation_prefix);
    let augmentation = EditAugmentation {
        cursor_byte_after: current_byte + insertion.len(),
        replace_range: Some(current_byte..current_byte),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

/// Setext 标题内的无选区 Enter：保留标题源码，在下划线之后建立块边界。
fn setext_heading_enter_augmentation(
    source: &str,
    underline_end: usize,
    continuation_prefix: &str,
) -> EditAugmentation {
    if continuation_prefix.is_empty() {
        emit_block_break_at(source, underline_end)
    } else {
        emit_container_line_break(source, underline_end, continuation_prefix)
    }
}

/// 缩进代码块内 Enter：插入换行并续上分类阶段确定的代码内容前缀。
fn indented_code_block_enter_augmentation(
    source: &str,
    current_byte: usize,
    continuation_prefix: &str,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, current_byte);
    let insertion = format!("{newline}{continuation_prefix}");
    let aug = EditAugmentation {
        cursor_byte_after: current_byte + insertion.len(),
        insert_text: Some(insertion),
        ..Default::default()
    };
    debug_assert_augmentation(&aug, source);
    aug
}

/// 围栏代码块围栏行 Enter(光标可在围栏行任意位置,插入点恒为该行行尾):
/// - 开头围栏行(含 info string):行尾插入单个换行,光标进入代码体第一行;
/// - 闭合围栏行:行尾建立块边界,光标落到围栏外的新空段(退出代码块)。
fn fence_line_enter_augmentation(
    source: &str,
    line_content_end: usize,
    is_opening: bool,
) -> EditAugmentation {
    if !is_opening {
        return emit_block_break_at(source, line_content_end);
    }
    let newline = preferred_newline_sequence(source, line_content_end);
    let aug = EditAugmentation {
        cursor_byte_after: line_content_end + newline.len(),
        replace_range: Some(line_content_end..line_content_end),
        insert_text: Some(newline.to_owned()),
    };
    debug_assert_augmentation(&aug, source);
    aug
}

fn list_item_enter_augmentation(
    source: &str,
    current_byte: usize,
    indent: &str,
    continuation_marker: &str,
    continuation_content_prefix: &str,
    empty: bool,
) -> Option<EditAugmentation> {
    if empty {
        return emit_replace_current_line(source, current_byte, indent);
    }

    if let Some(boundary) = hard_break_boundary_after(source, current_byte) {
        let replaced =
            extend_range_over_prefix_or_indent(source, boundary, continuation_content_prefix);
        return Some(emit_marker_break_replacing(source, replaced, indent, continuation_marker));
    }

    if let Some(newline_width) = newline_sequence_width_at(source, current_byte) {
        let next_line_start = current_byte + newline_width;
        if !is_only_blank_separator_before_content(source, next_line_start)
            && !next_source_line_has_list_marker(source, next_line_start)
        {
            let replaced = extend_range_over_prefix_or_indent(
                source,
                current_byte..next_line_start,
                continuation_content_prefix,
            );
            return Some(emit_marker_break_replacing(
                source,
                replaced,
                indent,
                continuation_marker,
            ));
        }
    }

    if newline_sequence_width_before(source, current_byte).is_some()
        && !next_source_line_has_list_marker(source, current_byte)
    {
        let replaced = extend_range_over_prefix_or_indent(
            source,
            current_byte..current_byte,
            continuation_content_prefix,
        );
        return Some(emit_inline_marker_replacing(source, replaced, indent, continuation_marker));
    }

    let split_at = reversible_split_point(source, current_byte);
    let augmentation = if split_at == current_byte {
        emit_marker_break(source, current_byte, indent, continuation_marker)
    } else {
        emit_marker_break_at(source, split_at, indent, continuation_marker)
    };
    Some(preserve_inline_elements_at_split(source, split_at, augmentation))
}

fn is_only_blank_separator_before_content(source: &str, line_start: usize) -> bool {
    let Some(blank_width) = blank_source_line_width(source, line_start) else {
        return false;
    };
    let line_end = line_start + blank_width;
    let Some(newline_width) = newline_sequence_width_at(source, line_end) else {
        return false;
    };
    let following_start = line_end + newline_width;
    following_start < source.len() && blank_source_line_width(source, following_start).is_none()
}

fn blockquote_enter_augmentation(
    source: &str,
    current_byte: usize,
    continuation_prefix: &str,
    empty_parent_prefix: &str,
    empty: bool,
) -> Option<EditAugmentation> {
    if empty {
        return emit_replace_current_line(source, current_byte, empty_parent_prefix);
    }

    if let Some(boundary) = hard_break_boundary_after(source, current_byte) {
        let replaced = extend_range_over_prefix_or_indent(source, boundary, continuation_prefix);
        return Some(emit_marker_break_replacing(source, replaced, "", continuation_prefix));
    }

    if let Some(newline_width) = newline_sequence_width_at(source, current_byte) {
        let next_line_start = current_byte + newline_width;
        if !next_source_line_has_blockquote_marker(source, next_line_start) {
            let replaced = extend_range_over_prefix_or_indent(
                source,
                current_byte..next_line_start,
                continuation_prefix,
            );
            return Some(emit_marker_break_replacing(source, replaced, "", continuation_prefix));
        }
    }

    if newline_sequence_width_before(source, current_byte).is_some()
        && !next_source_line_has_blockquote_marker(source, current_byte)
    {
        let replaced = extend_range_over_prefix_or_indent(
            source,
            current_byte..current_byte,
            continuation_prefix,
        );
        return Some(emit_inline_marker_replacing(source, replaced, "", continuation_prefix));
    }

    let split_at = reversible_split_point(source, current_byte);
    let augmentation = if split_at == current_byte {
        emit_marker_break(source, current_byte, "", continuation_prefix)
    } else {
        emit_marker_break_at(source, split_at, "", continuation_prefix)
    };
    Some(preserve_inline_elements_at_split(source, split_at, augmentation))
}

fn extend_range_over_prefix(
    source: &str,
    mut range: std::ops::Range<usize>,
    prefix: &str,
) -> std::ops::Range<usize> {
    if source.get(range.end..).is_some_and(|suffix| suffix.starts_with(prefix)) {
        range.end += prefix.len();
    }
    range
}

fn extend_range_over_prefix_or_indent(
    source: &str,
    range: std::ops::Range<usize>,
    prefix: &str,
) -> std::ops::Range<usize> {
    let extended = extend_range_over_prefix(source, range.clone(), prefix);
    if extended.end > range.end {
        return extended;
    }

    let indentation_width = source[range.end..]
        .bytes()
        .take_while(|source_byte| matches!(*source_byte, b' ' | b'\t'))
        .count();
    range.start..range.end + indentation_width
}

fn emit_inline_marker_replacing(
    source: &str,
    replace_range: std::ops::Range<usize>,
    indent: &str,
    marker: &str,
) -> EditAugmentation {
    let insertion = format!("{indent}{marker}");
    let augmentation = EditAugmentation {
        cursor_byte_after: replace_range.start + insertion.len(),
        replace_range: Some(replace_range),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

fn next_source_line_marker_start(source: &str, line_start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut marker_start = line_start;
    let mut leading_space_count = 0;
    while bytes.get(marker_start) == Some(&b' ') && leading_space_count < MAX_LEADING_BLOCK_INDENT {
        marker_start += 1;
        leading_space_count += 1;
    }
    (marker_start <= source.len()).then_some(marker_start)
}

fn next_source_line_has_list_marker(source: &str, line_start: usize) -> bool {
    next_source_line_marker_start(source, line_start)
        .is_some_and(|marker_start| parse_list_marker(source, marker_start).is_some())
}

fn next_source_line_has_blockquote_marker(source: &str, line_start: usize) -> bool {
    next_source_line_marker_start(source, line_start)
        .is_some_and(|marker_start| source.as_bytes().get(marker_start) == Some(&b'>'))
}

fn reversible_split_point(source: &str, current_byte: usize) -> usize {
    let bytes = source.as_bytes();
    if bytes.get(current_byte) == Some(&b' ') {
        let belongs_to_space_run = current_byte > 0 && bytes.get(current_byte - 1) == Some(&b' ')
            || bytes.get(current_byte + 1) == Some(&b' ');
        if !belongs_to_space_run {
            return current_byte + 1;
        }
    }
    current_byte
}

#[derive(Debug)]
struct InlineFrameDraft {
    source_range: std::ops::Range<usize>,
    content_start: Option<usize>,
    content_end: usize,
    kind: InlineFrameKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InlineFrameKind {
    Emphasis,
    Strong,
    Strikethrough,
    Link,
    Code,
}

#[derive(Debug)]
struct InlineSplitFrame<'a> {
    opening: &'a str,
    closing: &'a str,
}

#[derive(Debug)]
struct InlineWrapperFrame<'a> {
    source_range: std::ops::Range<usize>,
    content_start: usize,
    content_end: usize,
    opening: &'a str,
    closing: &'a str,
    kind: InlineFrameKind,
}

fn preserve_inline_elements_at_split(
    source: &str,
    split_at: usize,
    mut augmentation: EditAugmentation,
) -> EditAugmentation {
    let inline_split_at = inline_safe_split_point(source, split_at);
    let frames = inline_split_frames(source, inline_split_at);
    if frames.is_empty() {
        return augmentation;
    }

    let closing_len = frames.iter().map(|frame| frame.closing.len()).sum::<usize>();
    let opening_len = frames.iter().map(|frame| frame.opening.len()).sum::<usize>();
    let original_insertion = augmentation.insert_text.take().unwrap_or_default();
    let preserved_separator = source.get(inline_split_at..split_at).unwrap_or_default();
    let mut insertion = String::with_capacity(
        closing_len + preserved_separator.len() + original_insertion.len() + opening_len,
    );
    for frame in &frames {
        insertion.push_str(frame.closing);
    }
    insertion.push_str(preserved_separator);
    insertion.push_str(&original_insertion);
    for frame in frames.iter().rev() {
        insertion.push_str(frame.opening);
    }

    augmentation.insert_text = Some(insertion);
    if inline_split_at < split_at {
        let replace_range = augmentation.replace_range.get_or_insert(split_at..split_at);
        if replace_range.start == split_at {
            replace_range.start = inline_split_at;
        }
    }
    augmentation.cursor_byte_after += closing_len + opening_len;
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

fn inline_safe_split_point(source: &str, split_at: usize) -> usize {
    let Some(previous_byte) = split_at.checked_sub(1) else {
        return split_at;
    };
    let bytes = source.as_bytes();
    if bytes.get(previous_byte) != Some(&b' ') {
        return split_at;
    }
    let belongs_to_space_run = previous_byte > 0 && bytes.get(previous_byte - 1) == Some(&b' ')
        || bytes.get(split_at) == Some(&b' ');
    if belongs_to_space_run { split_at } else { previous_byte }
}

fn inline_split_frames(source: &str, split_at: usize) -> Vec<InlineSplitFrame<'_>> {
    inline_wrapper_frames(source)
        .into_iter()
        .filter(|frame| split_at >= frame.content_start && split_at <= frame.content_end)
        .map(|frame| InlineSplitFrame { opening: frame.opening, closing: frame.closing })
        .collect()
}

fn inline_wrapper_frames(source: &str) -> Vec<InlineWrapperFrame<'_>> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    let mut drafts = Vec::<InlineFrameDraft>::new();
    let mut completed = Vec::new();
    for (event, source_range) in
        Parser::new_ext(source, crate::parser::markdown_options()).into_offset_iter()
    {
        match event {
            Event::Start(Tag::Emphasis) => drafts.push(InlineFrameDraft {
                source_range,
                content_start: None,
                content_end: 0,
                kind: InlineFrameKind::Emphasis,
            }),
            Event::Start(Tag::Strong) => drafts.push(InlineFrameDraft {
                source_range,
                content_start: None,
                content_end: 0,
                kind: InlineFrameKind::Strong,
            }),
            Event::Start(Tag::Strikethrough) => drafts.push(InlineFrameDraft {
                source_range,
                content_start: None,
                content_end: 0,
                kind: InlineFrameKind::Strikethrough,
            }),
            Event::Start(Tag::Link { .. }) => drafts.push(InlineFrameDraft {
                source_range,
                content_start: None,
                content_end: 0,
                kind: InlineFrameKind::Link,
            }),
            Event::Text(_) | Event::InlineHtml(_) | Event::Html(_) => {
                record_inline_content(&mut drafts, &source_range);
            }
            Event::Code(code) => {
                record_inline_content(&mut drafts, &source_range);
                if let Some(frame) = inline_code_wrapper_frame(source, source_range, &code) {
                    completed.push(frame);
                }
            }
            Event::End(
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link,
            ) => {
                let Some(draft) = drafts.pop() else {
                    continue;
                };
                let Some(content_start) = draft.content_start else {
                    continue;
                };
                let Some(opening) = source.get(draft.source_range.start..content_start) else {
                    continue;
                };
                let Some(closing) = source.get(draft.content_end..draft.source_range.end) else {
                    continue;
                };
                if !opening.is_empty() && !closing.is_empty() {
                    completed.push(InlineWrapperFrame {
                        source_range: draft.source_range,
                        content_start,
                        content_end: draft.content_end,
                        opening,
                        closing,
                        kind: draft.kind,
                    });
                }
            }
            _ => {}
        }
    }
    completed
}

fn record_inline_content(drafts: &mut [InlineFrameDraft], source_range: &std::ops::Range<usize>) {
    for draft in drafts {
        draft.content_start = Some(
            draft.content_start.map_or(source_range.start, |start| start.min(source_range.start)),
        );
        draft.content_end = draft.content_end.max(source_range.end);
    }
}

fn inline_code_wrapper_frame<'a>(
    source: &'a str,
    source_range: std::ops::Range<usize>,
    code: &str,
) -> Option<InlineWrapperFrame<'a>> {
    let code_source = source.get(source_range.clone())?;
    let content_offset = code_source.find(code)?;
    let content_start = source_range.start + content_offset;
    let content_end = content_start + code.len();
    let opening = source.get(source_range.start..content_start)?;
    let closing = source.get(content_end..source_range.end)?;
    (!opening.is_empty() && !closing.is_empty()).then_some(InlineWrapperFrame {
        source_range,
        content_start,
        content_end,
        opening,
        closing,
        kind: InlineFrameKind::Code,
    })
}

fn cursor_touches_source_newline(source: &str, current_byte: usize) -> bool {
    newline_sequence_width_at(source, current_byte).is_some()
        || newline_sequence_width_before(source, current_byte).is_some()
}

// ─── 分类器 ────────────────────────────────────────────────────────────────

/// Enter 键的上下文分类。分类结果决定 `enter_context_augmentation` 的分支。
#[derive(Clone, Copy, Debug)]
pub enum HeadingPosition {
    Start { insert_at: usize },
    Interior,
    End { insert_at: usize },
}

#[derive(Debug)]
pub enum EnterContext {
    TopLevelParagraphEnd,
    ParagraphInterior,
    ParagraphStart {
        insert_at: usize,
    },
    Heading {
        level: u8,
        position: HeadingPosition,
        continuation_prefix: String,
    },
    /// Setext 标题；`underline_end` 不含下划线后的源码换行。
    SetextHeading {
        position: HeadingPosition,
        underline_end: usize,
        continuation_prefix: String,
    },
    ListItem {
        indent: String,
        continuation_marker: String,
        content_prefix: String,
        empty: bool,
        at_end: bool,
    },
    BlockQuoteLine {
        continuation_prefix: String,
        empty_parent_prefix: String,
        empty: bool,
    },
    CodeBlock,
    /// 围栏代码块的围栏行(开头或闭合);`line_content_end` 是该围栏行的
    /// 源码内容末尾(不含换行)。
    CodeBlockFenceLine {
        is_opening: bool,
        line_content_end: usize,
    },
    /// 缩进代码块及 Enter 后新行应继承的源码前缀。
    IndentedCodeBlock {
        continuation_prefix: String,
    },
    /// Metadata or block HTML: Markdown hard-break syntax is not valid here.
    LiteralBlockLine {
        continuation_prefix: String,
    },
    TableCell {
        next_cell_start: Option<usize>,
        column_count: usize,
        row_is_empty: bool,
        is_header_row: bool,
        row_line_end: usize,
        container_prefix: String,
    },
    EmptyBlockSeparatorLine,
    Other,
}

struct ItemFrame {
    start: usize,
    marker_end: usize,
    indent: String,
    continuation_marker: String,
    content_prefix: String,
    saw_content: bool,
}

struct TableFrame {
    cell_ranges: Vec<Vec<std::ops::Range<usize>>>,
    table_end: usize,
    container_prefix: String,
}

struct CodeBlockFrame {
    range: std::ops::Range<usize>,
    is_indented: bool,
}

fn classify_heading_hit(
    source: &str,
    current_byte: usize,
    level: u8,
    start: usize,
    range: &std::ops::Range<usize>,
) -> EnterContext {
    // pulldown-cmark 对 ATX 与 setext 标题都发 `Tag::Heading`；两种源码形态必须
    // 分流，否则 setext 会被错误套用 `# ` 前缀语义。
    let end = content_end_without_trailing_newline(source, start..range.end);
    if !heading_source_is_atx(source, start) {
        if current_byte >= start && current_byte <= end {
            return EnterContext::SetextHeading {
                position: if is_visible_content_start(source, start, current_byte) {
                    HeadingPosition::Start { insert_at: start }
                } else {
                    HeadingPosition::Interior
                },
                underline_end: end,
                continuation_prefix: canonical_container_prefix(source, start),
            };
        }
        return EnterContext::Other;
    }
    // 内容起点按实际源码扫描:至多 3 个前导空格 + `#` 序列 + 任意空白(空格/Tab),
    // 不能假定 `#` 后恰好一个空格(`#  Title`、`#\tTitle` 会落空在空白内)。
    let bytes = source.as_bytes();
    let mut probe = start;
    let mut leading_spaces = 0;
    while bytes.get(probe) == Some(&b' ') && leading_spaces < MAX_LEADING_BLOCK_INDENT {
        probe += 1;
        leading_spaces += 1;
    }
    probe += bytes[probe..].iter().take_while(|&&byte| byte == b'#').count();
    while matches!(bytes.get(probe), Some(b' ' | b'\t')) {
        probe += 1;
    }
    let content_start = probe.min(end);
    let position = if current_byte == end || heading_ends_at(source, start, current_byte) {
        HeadingPosition::End { insert_at: end }
    } else if is_visible_content_start(source, content_start, current_byte) {
        HeadingPosition::Start { insert_at: start }
    } else {
        HeadingPosition::Interior
    };
    if current_byte >= content_start && current_byte <= end {
        EnterContext::Heading {
            level,
            position,
            continuation_prefix: canonical_container_prefix(source, start),
        }
    } else {
        EnterContext::Other
    }
}

fn heading_ends_at(source: &str, heading_start: usize, cursor: usize) -> bool {
    fn find_heading(blocks: &[crate::builder::BlockNode], start: usize, cursor: usize) -> bool {
        for block in blocks {
            if block.block_range.start == start
                && matches!(block.kind, crate::builder::BlockKind::Heading { .. })
            {
                return editable_paragraph_navigation::has_text_boundary(
                    std::slice::from_ref(block),
                    cursor,
                    editable_paragraph_navigation::Boundary::End,
                );
            }
            if find_heading(&block.children, start, cursor) {
                return true;
            }
        }
        false
    }
    let document =
        crate::builder::MarkdownDoc::build_structure(&crate::parser::parse_markdown(source));
    find_heading(&document.blocks, heading_start, cursor)
}

/// 标题 range 起始处的源码必须是合法的 ATX marker：至多 3 个前导空格，
/// 1-6 个 `#`，后跟空格/制表符/行尾。
pub(crate) fn heading_source_is_atx(source: &str, heading_start: usize) -> bool {
    let bytes = source.as_bytes();
    let mut marker_probe = heading_start;
    let mut leading_spaces = 0;
    while bytes.get(marker_probe) == Some(&b' ') && leading_spaces < MAX_LEADING_BLOCK_INDENT {
        marker_probe += 1;
        leading_spaces += 1;
    }
    let hash_count = bytes[marker_probe..].iter().take_while(|&&byte| byte == b'#').count();
    if !(1..=6).contains(&hash_count) {
        return false;
    }
    matches!(bytes.get(marker_probe + hash_count), None | Some(b' ' | b'\t' | b'\r' | b'\n'))
}

/// 光标位于围栏代码块的围栏行时返回对应上下文,否则返回 `None`。
///
/// 开头围栏行 = 块起始所在源码行;闭合围栏行 = 块内容末尾所在行,且必须
/// 确实是合法闭合围栏(未闭合、延伸到 EOF 的代码块没有闭合围栏行)。
fn classify_fence_line_hit(
    source: &str,
    current_byte: usize,
    block_start: usize,
    block_content_end: usize,
) -> Option<EnterContext> {
    let (opening_line_start, _, opening_line_end) = locate_source_line_bounds(source, block_start)?;
    let opening_content_end = source_line_content_end(source, opening_line_end);
    let opening_position = LineColumnPosition::at_byte(source, opening_line_start, block_start)?;
    let (fence_char, opening_fence_len) =
        opening_fence_signature(source, opening_position, opening_content_end)?;
    if current_byte >= opening_line_start && current_byte <= opening_content_end {
        return Some(EnterContext::CodeBlockFenceLine {
            is_opening: true,
            line_content_end: opening_content_end,
        });
    }

    let (closing_line_start, _, closing_line_end) =
        locate_source_line_bounds(source, block_content_end)?;
    if closing_line_start == opening_line_start {
        return None;
    }
    let closing_content_end = source_line_content_end(source, closing_line_end);
    let closing_position = LineColumnPosition::at_line_start(closing_line_start);
    if !line_is_closing_fence(
        source,
        closing_position,
        closing_content_end,
        fence_char,
        opening_fence_len,
    ) {
        return None;
    }
    if current_byte >= closing_line_start && current_byte <= closing_content_end {
        return Some(EnterContext::CodeBlockFenceLine {
            is_opening: false,
            line_content_end: closing_content_end,
        });
    }
    None
}

#[derive(Clone, Debug)]
struct FencedBoundaryFrame {
    marker_start: usize,
    opening_position: LineColumnPosition,
    container_path: ContainerPath,
}

struct EnclosingFencedBoundary {
    frame: FencedBoundaryFrame,
    block_end: usize,
}

/// 该冷路径只在 CodeBlock 行尾 Delete 上触发。公开分类 API 不携带 parser frame，
/// 因此这里独立解析一次以取得 opener 与容器栈，避免扩大 EnterContext 公共形状。
fn line_closes_enclosing_fenced_block(
    source: &str,
    current_byte: usize,
    candidate_line_start: usize,
) -> bool {
    let Some(boundary) = enclosing_fenced_boundary(source, current_byte) else {
        return false;
    };
    candidate_line_closes_fenced_boundary(source, candidate_line_start, &boundary)
}

fn enclosing_fenced_boundary(source: &str, current_byte: usize) -> Option<EnclosingFencedBoundary> {
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

    let parser = Parser::new_ext(source, crate::parser::markdown_options());
    let mut item_container_paths = Vec::new();
    let mut fenced_block = None;
    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::Item) => {
                let container_path = parse_list_marker(source, range.start)
                    .map(|(_, content_start)| {
                        container_path(&canonical_container_prefix(source, content_start))
                    })
                    .or_else(|| item_container_paths.last().cloned())
                    .unwrap_or_default();
                item_container_paths.push(container_path);
            }
            Event::End(TagEnd::Item) => {
                item_container_paths.pop();
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => {
                fenced_block =
                    fenced_boundary_frame(source, range.start, item_container_paths.last());
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)) => fenced_block = None,
            Event::End(TagEnd::CodeBlock) => {
                let Some(frame) = fenced_block.take() else {
                    continue;
                };
                let content_end =
                    content_end_without_trailing_newline(source, frame.marker_start..range.end);
                if current_byte >= frame.marker_start && current_byte <= content_end {
                    return Some(EnclosingFencedBoundary { frame, block_end: range.end });
                }
            }
            _ => {}
        }
    }
    None
}

fn fenced_boundary_frame(
    source: &str,
    marker_start: usize,
    item_container_path: Option<&ContainerPath>,
) -> Option<FencedBoundaryFrame> {
    let (opening_line_start, _, opening_line_end) =
        locate_source_line_bounds(source, marker_start)?;
    let opening_content_end = source_line_content_end(source, opening_line_end);
    let base_path = item_container_path.cloned().unwrap_or_default();
    let container_path =
        source_container_path(source, opening_line_start, marker_start, &base_path);
    let opening_position = matching_container_position(
        source,
        opening_line_start,
        opening_content_end,
        &container_path,
    )
    .or_else(|| LineColumnPosition::at_byte(source, opening_line_start, marker_start))?;
    Some(FencedBoundaryFrame { marker_start, opening_position, container_path })
}

fn candidate_line_closes_fenced_boundary(
    source: &str,
    candidate_line_start: usize,
    boundary: &EnclosingFencedBoundary,
) -> bool {
    let Some((_, _, opening_line_end)) =
        locate_source_line_bounds(source, boundary.frame.marker_start)
    else {
        return false;
    };
    let opening_content_end = source_line_content_end(source, opening_line_end);
    let Some((fence_char, opening_fence_len)) =
        opening_fence_signature(source, boundary.frame.opening_position, opening_content_end)
    else {
        return false;
    };
    let Some((actual_line_start, _, candidate_line_end)) =
        locate_source_line_bounds(source, candidate_line_start)
    else {
        return false;
    };
    if actual_line_start != candidate_line_start
        || candidate_line_start < boundary.frame.marker_start
    {
        return false;
    }
    let candidate_content_end = source_line_content_end(source, candidate_line_end);
    if candidate_content_end > boundary.block_end {
        return false;
    }
    let Some(candidate_position) = matching_container_position(
        source,
        candidate_line_start,
        candidate_content_end,
        &boundary.frame.container_path,
    ) else {
        return false;
    };
    line_is_closing_fence(
        source,
        candidate_position,
        candidate_content_end,
        fence_char,
        opening_fence_len,
    )
}

fn container_path(prefix: &str) -> ContainerPath {
    let bytes = prefix.as_bytes();
    let mut segments = Vec::new();
    let mut probe = 0;
    let mut absolute_column = 0;
    while probe < bytes.len() {
        let whitespace_start = probe;
        let whitespace_start_column = absolute_column;
        while matches!(bytes.get(probe), Some(b' ' | b'\t')) {
            absolute_column = if bytes[probe] == b'\t' {
                next_markdown_tab_stop(absolute_column)
            } else {
                absolute_column + 1
            };
            probe += 1;
        }
        if whitespace_start < probe {
            let width = absolute_column - whitespace_start_column;
            let precedes_quote = bytes.get(probe) == Some(&b'>');
            let is_optional_quote_indent = precedes_quote
                && width <= MAX_LEADING_BLOCK_INDENT
                && bytes[whitespace_start..probe].iter().all(|byte| *byte == b' ');
            if !is_optional_quote_indent {
                segments.push(ContainerSegment::IndentColumns(width));
            }
        }
        if bytes.get(probe) != Some(&b'>') {
            break;
        }
        segments.push(ContainerSegment::BlockQuote);
        probe += 1;
        absolute_column += 1;
        if matches!(bytes.get(probe), Some(b' ' | b'\t')) {
            absolute_column = if bytes[probe] == b'\t' {
                next_markdown_tab_stop(absolute_column)
            } else {
                absolute_column + 1
            };
            probe += 1;
        }
    }
    ContainerPath(segments)
}

fn source_container_path(
    source: &str,
    line_start: usize,
    prefix_end: usize,
    base_path: &ContainerPath,
) -> ContainerPath {
    if !base_path.0.is_empty()
        && let Some(position) =
            matching_container_position(source, line_start, prefix_end, base_path)
    {
        return append_source_block_quotes(source, prefix_end, base_path.clone(), position);
    }
    explicit_source_container_path(source, line_start, prefix_end)
}

fn append_source_block_quotes(
    source: &str,
    prefix_end: usize,
    mut path: ContainerPath,
    mut position: LineColumnPosition,
) -> ContainerPath {
    while position.byte_offset < prefix_end {
        let Some(next_position) = match_block_quote_segment(source, prefix_end, position) else {
            break;
        };
        if next_position.byte_offset <= position.byte_offset {
            break;
        }
        path.0.push(ContainerSegment::BlockQuote);
        position = next_position;
    }
    path
}

fn explicit_source_container_path(
    source: &str,
    line_start: usize,
    prefix_end: usize,
) -> ContainerPath {
    let bytes = source.as_bytes();
    let mut path = ContainerPath::default();
    let mut position = LineColumnPosition::at_line_start(line_start);
    while position.byte_offset < prefix_end {
        let mut marker_probe = position.byte_offset;
        let mut leading_spaces = 0;
        while marker_probe < prefix_end
            && bytes.get(marker_probe) == Some(&b' ')
            && leading_spaces < MAX_LEADING_BLOCK_INDENT
        {
            marker_probe += 1;
            leading_spaces += 1;
        }

        if let Some((_, content_start)) = parse_list_marker(source, marker_probe)
            && content_start <= prefix_end
            && let Some(content_position) =
                LineColumnPosition::at_byte(source, line_start, content_start)
            && let Some(required_columns) =
                content_position.absolute_column.checked_sub(position.absolute_column)
        {
            path.0.push(ContainerSegment::IndentColumns(required_columns));
            position = content_position;
            continue;
        }

        let Some(next_position) = match_block_quote_segment(source, prefix_end, position) else {
            break;
        };
        path.0.push(ContainerSegment::BlockQuote);
        position = next_position;
    }
    path
}

fn matching_container_position(
    source: &str,
    line_start: usize,
    line_end: usize,
    path: &ContainerPath,
) -> Option<LineColumnPosition> {
    let mut position = LineColumnPosition::at_line_start(line_start);
    for segment in &path.0 {
        position = match *segment {
            ContainerSegment::BlockQuote => match_block_quote_segment(source, line_end, position)?,
            ContainerSegment::IndentColumns(required_width) => {
                advance_container_columns(source, line_end, position, required_width)?
            }
        };
    }
    Some(position)
}

fn match_block_quote_segment(
    source: &str,
    line_end: usize,
    mut position: LineColumnPosition,
) -> Option<LineColumnPosition> {
    let mut leading_spaces = position.leaf_indent_columns()?;
    if leading_spaces > MAX_LEADING_BLOCK_INDENT {
        return None;
    }
    // 上一容器段的跨界 Tab residual 等价于引用 marker 前的可选缩进；
    // 从此处起它已属于容器路径，不能继续作为叶块缩进传播。
    position.container_target_column = position.absolute_column;

    let bytes = source.as_bytes();
    while position.byte_offset < line_end
        && bytes.get(position.byte_offset) == Some(&b' ')
        && leading_spaces < MAX_LEADING_BLOCK_INDENT
    {
        position.byte_offset += 1;
        position.absolute_column += 1;
        position.container_target_column += 1;
        leading_spaces += 1;
    }
    if bytes.get(position.byte_offset) != Some(&b'>') {
        return None;
    }
    position.byte_offset += 1;
    position.absolute_column += 1;
    position.container_target_column += 1;

    if position.byte_offset < line_end {
        match bytes[position.byte_offset] {
            b' ' => {
                position.byte_offset += 1;
                position.absolute_column += 1;
                position.container_target_column += 1;
            }
            b'\t' => {
                position.byte_offset += 1;
                position.absolute_column = next_markdown_tab_stop(position.absolute_column);
                position.container_target_column += 1;
            }
            _ => {}
        }
    }
    Some(position)
}

fn advance_container_columns(
    source: &str,
    line_end: usize,
    mut position: LineColumnPosition,
    required_columns: usize,
) -> Option<LineColumnPosition> {
    position.container_target_column =
        position.container_target_column.checked_add(required_columns)?;
    // Tab 可以一次越过目标列；多出的 1-3 列由叶块缩进继续使用。
    while position.absolute_column < position.container_target_column {
        position = advance_whitespace_byte(source, line_end, position)?;
    }
    Some(position)
}

fn advance_whitespace_byte(
    source: &str,
    line_end: usize,
    mut position: LineColumnPosition,
) -> Option<LineColumnPosition> {
    if position.byte_offset >= line_end {
        return None;
    }
    match source.as_bytes()[position.byte_offset] {
        b' ' => position.absolute_column += 1,
        b'\t' => position.absolute_column = next_markdown_tab_stop(position.absolute_column),
        _ => return None,
    }
    position.byte_offset += 1;
    Some(position)
}

fn marker_position_after_optional_indent(
    source: &str,
    line_end: usize,
    mut position: LineColumnPosition,
) -> Option<LineColumnPosition> {
    let mut leaf_indent_columns = position.leaf_indent_columns()?;
    if leaf_indent_columns > MAX_LEADING_BLOCK_INDENT {
        return None;
    }
    while position.byte_offset < line_end
        && matches!(source.as_bytes()[position.byte_offset], b' ' | b'\t')
    {
        // 容器目标已经满足后才开始的 Tab 属于叶块，必须从叶块列重新展开为 4 列；
        // 只有上面的 residual 才能表示同一 Tab 跨容器后留下的虚拟列。
        leaf_indent_columns = if source.as_bytes()[position.byte_offset] == b'\t' {
            next_markdown_tab_stop(leaf_indent_columns)
        } else {
            leaf_indent_columns + 1
        };
        position = advance_whitespace_byte(source, line_end, position)?;
        if leaf_indent_columns > MAX_LEADING_BLOCK_INDENT {
            return None;
        }
    }
    Some(position)
}

/// 扫描开头围栏行,返回(围栏字符, 围栏长度)。非法围栏返回 `None`。
fn opening_fence_signature(
    source: &str,
    line_position: LineColumnPosition,
    content_end: usize,
) -> Option<(u8, usize)> {
    let bytes = source.as_bytes();
    let fence_start =
        marker_position_after_optional_indent(source, content_end, line_position)?.byte_offset;
    let fence_char = *bytes.get(fence_start).filter(|byte| matches!(byte, b'`' | b'~'))?;
    let fence_len =
        bytes[fence_start..content_end].iter().take_while(|&&byte| byte == fence_char).count();
    (fence_len >= MIN_FENCE_MARKER_LENGTH).then_some((fence_char, fence_len))
}

/// 该行是否为合法闭合围栏:至多 3 个前导空格,≥ 开头围栏长度的相同围栏字符,
/// 其余位置只允许空白。未闭合代码块的末行(代码内容)不会通过此检查。
fn line_is_closing_fence(
    source: &str,
    line_position: LineColumnPosition,
    content_end: usize,
    fence_char: u8,
    opening_fence_len: usize,
) -> bool {
    let bytes = source.as_bytes();
    let Some(fence_start) =
        marker_position_after_optional_indent(source, content_end, line_position)
            .map(|position| position.byte_offset)
    else {
        return false;
    };
    let fence_len =
        bytes[fence_start..content_end].iter().take_while(|&&byte| byte == fence_char).count();
    if fence_len < opening_fence_len {
        return false;
    }
    bytes[fence_start + fence_len..content_end].iter().all(|byte| matches!(byte, b' ' | b'\t'))
}

pub fn classify_enter_context(source: &str, current_byte: usize) -> EnterContext {
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

    #[cfg(test)]
    CLASSIFY_PARSE_COUNT.with(|parse_count| parse_count.set(parse_count.get() + 1));

    let mark_item_content_seen = |stack: &mut Vec<ItemFrame>| {
        if let Some(frame) = stack.last_mut() {
            frame.saw_content = true;
        }
    };

    let parser = Parser::new_ext(source, crate::parser::markdown_options());
    let mut item_stack: Vec<ItemFrame> = Vec::new();
    let mut table: Option<TableFrame> = None;
    let mut heading_level: Option<u8> = None;
    let mut heading_start: Option<usize> = None;
    let mut paragraph_start: Option<(usize, usize, usize)> = None;
    let mut code_block: Option<CodeBlockFrame> = None;
    let mut blockquote_depth: usize = 0;
    let mut matched_list_item: Option<EnterContext> = None;
    let mut matched_literal_block: Option<EnterContext> = None;

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::Item) => {
                // 子 item 本身也是父 item 的内容:只挂子列表的父 item 不算空,
                // 否则父行回车会走"空 item 退出"删掉 marker、把子列表提升为顶层。
                mark_item_content_seen(&mut item_stack);
                let indent = list_item_indent(source, range.start);
                if let Some((bullet, marker_end)) = parse_list_marker(source, range.start) {
                    item_stack.push(ItemFrame {
                        start: range.start,
                        marker_end,
                        indent,
                        continuation_marker: list_continuation_marker(
                            source,
                            range.start,
                            marker_end,
                            bullet,
                        ),
                        content_prefix: list_item_content_prefix(source, range.start, marker_end),
                        saw_content: false,
                    });
                } else {
                    let content_prefix = format!("{indent}  ");
                    item_stack.push(ItemFrame {
                        start: range.start,
                        marker_end: range.start,
                        indent,
                        continuation_marker: String::from("- "),
                        content_prefix,
                        saw_content: false,
                    });
                }
            }
            Event::End(TagEnd::Item) => {
                if let Some(frame) = item_stack.pop() {
                    // item range 含尾部换行 run（块间空行分隔也算在内）；
                    // 命中测试用去尾换行的内容末尾，否则紧随其后的顶层块首字节
                    // 会被误判为仍属于该 item。
                    let end =
                        content_end_without_trailing_newline(source, frame.marker_end..range.end);
                    // 光标在 marker 内部(`-| item`)时按内容起点处理,而不是落到
                    // Other 产生 `-\n item` 这样的懒延续残留。
                    if frame.marker_end > frame.start
                        && current_byte > frame.start
                        && current_byte <= end
                    {
                        // 光标所在行为纯空白(如嵌套空 item 退出后的残留缩进行)时,
                        // 即使 item 其他行有内容也按空 item 处理:回车应继续退出,
                        // 而不是在空白行下意外创建同级 item。
                        let blank_line_width = blank_source_line_width(source, current_byte);
                        let empty = !frame.saw_content || blank_line_width.is_some();
                        // 残留空白行可能同时落在多级 item 的 range 内;选择缩进严格
                        // 窄于该行空白宽度的最深帧——帧缩进与行内容等宽时原地替换
                        // 是不动点,无法真正"退一层"。
                        let frame_is_exit_target = match blank_line_width {
                            Some(width) => frame.indent.len() < width,
                            None => true,
                        };
                        let at_end = current_byte == end;
                        if frame_is_exit_target && matched_list_item.is_none() {
                            matched_list_item = Some(EnterContext::ListItem {
                                indent: frame.indent,
                                continuation_marker: frame.continuation_marker,
                                content_prefix: frame.content_prefix,
                                empty,
                                at_end,
                            });
                        }
                    }
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                blockquote_depth += 1;
                mark_item_content_seen(&mut item_stack);
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                blockquote_depth = blockquote_depth.saturating_sub(1);
            }
            Event::Start(Tag::Heading { level, .. }) => {
                mark_item_content_seen(&mut item_stack);
                heading_level = Some(level as u8);
                heading_start = Some(range.start);
            }
            Event::End(TagEnd::Heading(_)) => {
                if let (Some(level), Some(start)) = (heading_level.take(), heading_start.take())
                    && current_byte >= start
                    && current_byte <= range.end
                {
                    let res = classify_heading_hit(source, current_byte, level, start, &range);
                    if !matches!(res, EnterContext::Other) {
                        return res;
                    }
                }
            }
            Event::Start(Tag::Paragraph) => {
                mark_item_content_seen(&mut item_stack);
                paragraph_start = Some((range.start, item_stack.len(), blockquote_depth));
            }
            Event::End(TagEnd::Paragraph) => {
                if let Some((start, p_item_depth, p_bq_depth)) = paragraph_start.take()
                    && current_byte >= start
                    && current_byte <= range.end
                    && p_item_depth == 0
                    && p_bq_depth == 0
                {
                    let end = content_end_without_trailing_newline(source, start..range.end);
                    if current_byte == end {
                        return EnterContext::TopLevelParagraphEnd;
                    }
                    if is_visible_content_start(source, start, current_byte) {
                        return EnterContext::ParagraphStart { insert_at: start };
                    }
                    if source_line_is_empty(source, current_byte) {
                        return EnterContext::EmptyBlockSeparatorLine;
                    }
                    return EnterContext::ParagraphInterior;
                }
            }
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
                    // 否则紧邻的下一块首字节会被误判为 CodeBlock。
                    let block_range = frame.range.start..range.end;
                    let end = content_end_without_trailing_newline(source, block_range.clone());
                    if current_byte >= frame.range.start && current_byte <= end {
                        if frame.is_indented {
                            let continuation_prefix = indented_code_continuation_prefix(
                                source,
                                current_byte,
                                &block_range,
                            );
                            return EnterContext::IndentedCodeBlock { continuation_prefix };
                        }
                        if let Some(fence_context) =
                            classify_fence_line_hit(source, current_byte, frame.range.start, end)
                        {
                            return fence_context;
                        }
                        return EnterContext::CodeBlock;
                    }
                }
            }
            Event::Start(Tag::MetadataBlock(_))
                if current_byte >= range.start && current_byte <= range.end =>
            {
                matched_literal_block = Some(EnterContext::LiteralBlockLine {
                    continuation_prefix: literal_line_continuation_prefix(source, current_byte),
                });
            }
            Event::Start(Tag::Table(_)) => {
                mark_item_content_seen(&mut item_stack);
                table = Some(TableFrame {
                    cell_ranges: Vec::new(),
                    table_end: range.end,
                    container_prefix: canonical_container_prefix(source, range.start),
                });
            }
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => {
                if let Some(t) = table.as_mut() {
                    t.cell_ranges.push(Vec::new());
                }
            }
            Event::Start(Tag::TableCell) => {
                if let Some(t) = table.as_mut()
                    && let Some(row) = t.cell_ranges.last_mut()
                {
                    row.push(range.start..range.end);
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(t) = table.as_mut()
                    && let Some(row) = t.cell_ranges.last_mut()
                    && let Some(cell) = row.last_mut()
                {
                    cell.end = range.end;
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(table_frame) = table.as_mut() {
                    table_frame.table_end = range.end;
                }
            }
            Event::Html(text) if !text.is_empty() => {
                mark_item_content_seen(&mut item_stack);
                if current_byte >= range.start && current_byte <= range.end {
                    matched_literal_block = Some(EnterContext::LiteralBlockLine {
                        continuation_prefix: literal_line_continuation_prefix(source, current_byte),
                    });
                }
            }
            Event::Text(text) | Event::Code(text) | Event::InlineHtml(text) if !text.is_empty() => {
                mark_item_content_seen(&mut item_stack);
            }
            _ => {}
        }
    }

    if let Some(t) = table {
        for (row_idx, row) in t.cell_ranges.iter().enumerate() {
            for (col_idx, cell) in row.iter().enumerate() {
                if current_byte >= cell.start && current_byte <= cell.end {
                    let next_cell_start = t
                        .cell_ranges
                        .get(row_idx + 1)
                        .and_then(|next_row| next_row.get(col_idx))
                        .map(|next_cell| table_cell_content_start(source, next_cell));
                    let column_count = row.len();
                    let row_is_empty = row.iter().all(|row_cell| {
                        source[row_cell.clone()]
                            .bytes()
                            .all(|byte| matches!(byte, b' ' | b'\t' | b'|' | b'\r' | b'\n'))
                    });
                    let is_header_row = row_idx == 0;
                    let last_cell_end = row.last().map_or(cell.end, |last_cell| last_cell.end);
                    let last_row_content_end = content_end_without_trailing_newline(
                        source,
                        0..t.table_end.max(last_cell_end),
                    );
                    let row_probe = if next_cell_start.is_none() {
                        last_row_content_end.saturating_sub(1)
                    } else {
                        last_cell_end.saturating_sub(1)
                    };
                    let row_line_end = locate_source_line_bounds(source, row_probe)
                        .map(|(_, _, line_end)| source_line_content_end(source, line_end))
                        .unwrap_or(last_cell_end);
                    return EnterContext::TableCell {
                        next_cell_start,
                        column_count,
                        row_is_empty,
                        is_header_row,
                        row_line_end,
                        container_prefix: t.container_prefix.clone(),
                    };
                }
            }
        }
    }

    if let Some(literal_block) = matched_literal_block {
        return literal_block;
    }

    if let Some((_line_start, content_start, line_end)) =
        locate_blockquote_line(source, current_byte)
        && let content_end = source_line_content_end(source, line_end)
        && current_byte >= content_start
        && current_byte <= content_end
    {
        let empty = content_start == content_end;
        return EnterContext::BlockQuoteLine {
            continuation_prefix: canonical_container_prefix(source, content_start),
            empty_parent_prefix: blockquote_parent_prefix(source, content_start),
            empty,
        };
    }

    if let Some(list_item) = matched_list_item {
        return list_item;
    }

    if source_line_is_empty(source, current_byte) {
        return EnterContext::EmptyBlockSeparatorLine;
    }

    EnterContext::Other
}

fn literal_line_continuation_prefix(source: &str, current_byte: usize) -> String {
    let Some((line_start, _, line_end)) = locate_source_line_bounds(source, current_byte) else {
        return String::new();
    };
    let content_start = container_content_start_on_line(source, line_start, line_end);
    let indentation_end = source.as_bytes()[content_start..line_end]
        .iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count()
        + content_start;
    format!(
        "{}{}",
        canonical_container_prefix(source, content_start),
        &source[content_start..indentation_end]
    )
}

fn source_line_is_empty(source: &str, byte: usize) -> bool {
    let Some((start, _, end)) = locate_source_line_bounds(source, byte) else {
        return false;
    };
    start == source_line_content_end(source, end)
}

/// 光标所在行为纯空白行(去 `\r` 后仅含空格/Tab)时返回其空白宽度,否则返回 `None`。
fn blank_source_line_width(source: &str, byte: usize) -> Option<usize> {
    let (start, _, end) = locate_source_line_bounds(source, byte)?;
    let content_end = source_line_content_end(source, end);
    let is_blank = source[start..content_end].bytes().all(|byte| matches!(byte, b' ' | b'\t'));
    is_blank.then_some(content_end - start)
}

fn source_line_content_end(source: &str, line_end: usize) -> usize {
    source[..line_end].strip_suffix('\r').map_or(line_end, |_| line_end - 1)
}

fn content_end_without_trailing_newline(source: &str, range: std::ops::Range<usize>) -> usize {
    let mut end = range.end.min(source.len());
    let start = range.start.min(end);
    let source_bytes = source.as_bytes();
    while end > start && matches!(source_bytes[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    end
}

fn line_indent_and_content(source: &str, line_start: usize, line_end: usize) -> (&str, bool) {
    let content_end = source_line_content_end(source, line_end);
    let indent_width = source[line_start..content_end]
        .bytes()
        .take_while(|source_byte| matches!(*source_byte, b' ' | b'\t'))
        .count();
    let indent_end = line_start + indent_width;
    (&source[line_start..indent_end], indent_end < content_end)
}

/// 计算缩进代码块续行前缀。空白行从同一块最近的非空行继承缩进。
fn indented_code_continuation_prefix(
    source: &str,
    current_byte: usize,
    block_range: &std::ops::Range<usize>,
) -> String {
    let Some((line_start, _, line_end)) = locate_source_line_bounds(source, current_byte) else {
        return String::new();
    };
    let (current_indent, current_has_content) =
        line_indent_and_content(source, line_start, line_end);
    if current_has_content {
        return current_indent.to_owned();
    }

    let mut previous_line_start = line_start;
    while let Some(newline_width) = newline_sequence_width_before(source, previous_line_start) {
        let previous_line_end = previous_line_start - newline_width;
        if previous_line_end <= block_range.start {
            break;
        }
        let candidate_start = source[..previous_line_end]
            .bytes()
            .rposition(|source_byte| source_byte == b'\n')
            .map_or(0, |newline| newline + 1);
        let (candidate_indent, candidate_has_content) =
            line_indent_and_content(source, candidate_start, previous_line_end);
        if candidate_has_content {
            return candidate_indent.to_owned();
        }
        previous_line_start = candidate_start;
    }

    let mut next_line_start = line_end + newline_sequence_width_at(source, line_end).unwrap_or(0);
    while next_line_start < block_range.end && next_line_start < source.len() {
        let next_line_end = source[next_line_start..]
            .bytes()
            .position(|source_byte| source_byte == b'\n')
            .map_or(source.len(), |newline| next_line_start + newline);
        let (candidate_indent, candidate_has_content) =
            line_indent_and_content(source, next_line_start, next_line_end);
        if candidate_has_content {
            return candidate_indent.to_owned();
        }
        let newline_width = newline_sequence_width_at(source, next_line_end).unwrap_or(0);
        if newline_width == 0 {
            break;
        }
        next_line_start = next_line_end + newline_width;
    }

    current_indent.to_owned()
}

/// 单元格正文起点；空单元格落在单元格源码范围末端。
pub(crate) fn table_cell_content_start(source: &str, cell_range: &std::ops::Range<usize>) -> usize {
    let leading_non_content = source[cell_range.clone()]
        .bytes()
        .take_while(|source_byte| matches!(*source_byte, b' ' | b'\t' | b'\r' | b'\n' | b'|'))
        .count();
    cell_range.start + leading_non_content
}

fn table_insert_row_augmentation(
    source: &str,
    row_line_end: usize,
    column_count: usize,
    container_prefix: &str,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, row_line_end);
    let mut row_source = String::from(container_prefix);
    row_source.push_str(&"|  ".repeat(column_count));
    row_source.push('|');

    let (insert_at, insertion, first_cell_content_offset) =
        if let Some(newline_width) = newline_sequence_width_at(source, row_line_end) {
            (row_line_end + newline_width, row_source, container_prefix.len() + 2)
        } else {
            let insertion = format!("{newline}{row_source}");
            (row_line_end, insertion, newline.len() + container_prefix.len() + 2)
        };
    let aug = EditAugmentation {
        cursor_byte_after: insert_at + first_cell_content_offset,
        replace_range: Some(insert_at..insert_at),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&aug, source);
    aug
}

fn table_exit_empty_row_augmentation(
    source: &str,
    row_line_end: usize,
    container_prefix: &str,
) -> Option<EditAugmentation> {
    let row_probe = row_line_end.saturating_sub(1);
    let (row_start, _, _) = locate_source_line_bounds(source, row_probe)?;
    let preceding_newline_width = newline_sequence_width_before(source, row_start)?;
    let replace_start = row_start - preceding_newline_width;
    let newline = preferred_newline_sequence(source, replace_start);
    let insertion = if container_prefix.is_empty() {
        newline.repeat(BLOCK_BOUNDARY_NEWLINE_COUNT)
    } else {
        format!("{newline}{container_prefix}")
    };
    let aug = EditAugmentation {
        cursor_byte_after: replace_start + insertion.len(),
        replace_range: Some(replace_start..row_line_end),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}

// ─── 光标位置附近的源码切分 ───────────────────────────────────────────────

fn get_marker_delete_range(source: &str, current_byte: usize) -> Option<std::ops::Range<usize>> {
    let (line_start, _, _) = locate_source_line_bounds(source, current_byte)?;
    if line_start == current_byte {
        return None;
    }
    let prefix = &source[line_start..current_byte];
    let trimmed = prefix.trim_start_matches(' ');

    let is_match = match trimmed {
        _ if trimmed.starts_with('>') => {
            trimmed.chars().all(|character| matches!(character, '>' | ' ' | '\t'))
        }
        _ if trimmed.starts_with('#') => {
            let hashes = trimmed.chars().take_while(|&c| c == '#').count();
            hashes <= 6 && hashes > 0 && marker_separator_is_complete(&trimmed[hashes..])
        }
        _ if trimmed.starts_with('-') || trimmed.starts_with('*') || trimmed.starts_with('+') => {
            unordered_marker_suffix_is_complete(&trimmed[1..])
        }
        _ => {
            let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
            digits > 0 && {
                let rest = &trimmed[digits..];
                rest.strip_prefix(['.', ')']).is_some_and(marker_separator_is_complete)
            }
        }
    };

    if is_match { Some(line_start..current_byte) } else { None }
}

fn marker_separator_is_complete(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| matches!(byte, b' ' | b'\t'))
}

fn unordered_marker_suffix_is_complete(suffix: &str) -> bool {
    let separator_end = suffix.bytes().take_while(|byte| matches!(byte, b' ' | b'\t')).count();
    if separator_end == 0 {
        return false;
    }
    let after_separator = &suffix[separator_end..];
    if after_separator.is_empty() {
        return true;
    }

    ["[ ]", "[x]", "[X]"].into_iter().any(|task_marker| {
        after_separator.strip_prefix(task_marker).is_some_and(|trailing| {
            trailing.is_empty() || trailing.bytes().all(|byte| matches!(byte, b' ' | b'\t'))
        })
    })
}

fn list_continuation_marker(
    source: &str,
    marker_start: usize,
    marker_end: usize,
    bullet: ListBullet,
) -> String {
    let source_marker = &source[marker_start..marker_end];
    match bullet {
        ListBullet::Bullet => source_marker.to_owned(),
        ListBullet::Ordered(number) => {
            let suffix_start = source_marker.bytes().take_while(u8::is_ascii_digit).count();
            format!("{}{}", number + 1, &source_marker[suffix_start..])
        }
        ListBullet::TaskList(checked) => {
            let mut continuation_marker = if checked {
                source_marker.replacen("[x]", "[ ]", 1).replacen("[X]", "[ ]", 1)
            } else {
                source_marker.to_owned()
            };
            if !matches!(continuation_marker.as_bytes().last(), Some(b' ' | b'\t')) {
                continuation_marker.push(TASK_LIST_CONTENT_SEPARATOR);
            }
            continuation_marker
        }
    }
}

fn locate_source_line_bounds(source: &str, byte: usize) -> Option<(usize, usize, usize)> {
    let bytes = source.as_bytes();
    let start = bytes[..byte].iter().rposition(|&b| b == b'\n').map(|p| p + 1).unwrap_or(0);
    let end =
        bytes[byte..].iter().position(|&b| b == b'\n').map(|p| byte + p).unwrap_or(source.len());
    Some((start, byte, end))
}

pub(crate) fn parse_list_marker(source: &str, marker_start: usize) -> Option<(ListBullet, usize)> {
    let bytes = source.as_bytes();
    if matches!(bytes.get(marker_start), Some(b'-' | b'+' | b'*')) {
        let content_start = list_marker_separator_end(source, marker_start + 1)?;

        let task_start = content_start;
        let Some(task_marker) = bytes.get(task_start..task_start + 3) else {
            return Some((ListBullet::Bullet, content_start));
        };
        let checked = match task_marker {
            b"[ ]" => false,
            b"[x]" | b"[X]" => true,
            _ => return Some((ListBullet::Bullet, content_start)),
        };
        let task_content_start =
            list_marker_separator_end(source, task_start + 3).unwrap_or(task_start + 3);
        return Some((ListBullet::TaskList(checked), task_content_start));
    }

    let mut marker_end = marker_start;
    while let Some(&b) = bytes.get(marker_end) {
        if b.is_ascii_digit() {
            marker_end += 1;
        } else {
            break;
        }
    }
    let digit_count = marker_end - marker_start;
    if !(1..=9).contains(&digit_count) || !matches!(bytes.get(marker_end), Some(b'.' | b')')) {
        return None;
    }

    let number = source[marker_start..marker_end].parse::<u64>().ok()?;
    let content_start = list_marker_separator_end(source, marker_end + 1)?;
    Some((ListBullet::Ordered(number), content_start))
}

fn list_marker_separator_end(source: &str, separator_start: usize) -> Option<usize> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, separator_start)?;
    let marker_column = markdown_column_after(&source[line_start..separator_start], 0);
    let maximum_content_column = marker_column + MARKDOWN_TAB_STOP_WIDTH;
    let mut column = marker_column;
    let mut probe = separator_start;

    while probe < line_end {
        let next_column = match source.as_bytes()[probe] {
            b' ' => column + 1,
            b'\t' => next_markdown_tab_stop(column),
            _ => break,
        };
        if next_column > maximum_content_column {
            break;
        }
        column = next_column;
        probe += 1;
    }

    (probe > separator_start).then_some(probe)
}

fn list_item_content_prefix(source: &str, marker_start: usize, content_start: usize) -> String {
    let Some((line_start, _, _)) = locate_source_line_bounds(source, marker_start) else {
        return String::new();
    };
    let indent = &source[line_start..marker_start];
    let marker_start_column = markdown_column_after(indent, 0);
    let content_column =
        markdown_column_after(&source[marker_start..content_start], marker_start_column);
    format!("{indent}{}", " ".repeat(content_column - marker_start_column))
}

fn markdown_column_after(text: &str, starting_column: usize) -> usize {
    text.bytes().fold(starting_column, |column, byte| {
        if byte == b'\t' { next_markdown_tab_stop(column) } else { column + 1 }
    })
}

fn next_markdown_tab_stop(column: usize) -> usize {
    (column / MARKDOWN_TAB_STOP_WIDTH + 1) * MARKDOWN_TAB_STOP_WIDTH
}

pub(crate) fn list_item_indent(source: &str, marker_start: usize) -> String {
    let Some((line_start, _, _)) = locate_source_line_bounds(source, marker_start) else {
        return String::new();
    };
    source[line_start..marker_start].to_string()
}

/// 返回叶块所在行的容器续行前缀。
///
/// 引用 marker 原样保留；列表 marker 改写为等宽空格，使新行留在原列表项内，
/// 而不会意外创建同级列表项。该表示可组合处理 `> - `、`- > ` 等嵌套路径。
fn canonical_container_prefix(source: &str, leaf_start: usize) -> String {
    let Some((line_start, _, line_end)) = locate_source_line_bounds(source, leaf_start) else {
        return String::new();
    };
    let prefix_end = leaf_start.min(line_end);
    let mut prefix = String::with_capacity(prefix_end.saturating_sub(line_start));
    let mut probe = line_start;

    while probe < prefix_end {
        if let Some((_, content_start)) = parse_list_marker(source, probe)
            && content_start <= prefix_end
        {
            let marker_start_column = markdown_column_after(&prefix, 0);
            let content_column =
                markdown_column_after(&source[probe..content_start], marker_start_column);
            prefix.extend(std::iter::repeat_n(' ', content_column - marker_start_column));
            probe = content_start;
            continue;
        }

        let Some(character) = source[probe..prefix_end].chars().next() else {
            break;
        };
        prefix.push(character);
        probe += character.len_utf8();
    }

    prefix
}

fn blockquote_parent_prefix(source: &str, content_start: usize) -> String {
    let Some((line_start, _, _)) = locate_source_line_bounds(source, content_start) else {
        return String::new();
    };
    let structural_prefix = &source[line_start..content_start];
    let Some(last_quote_offset) = structural_prefix.rfind('>') else {
        return String::new();
    };
    structural_prefix[..last_quote_offset].to_owned()
}

fn locate_blockquote_line(source: &str, byte: usize) -> Option<(usize, usize, usize)> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, byte)?;
    let bytes = source.as_bytes();
    let leading_spaces =
        bytes[line_start..line_end].iter().take_while(|source_byte| **source_byte == b' ').count();
    if leading_spaces > MAX_LEADING_BLOCK_INDENT {
        return None;
    }
    let mut content_start = line_start + leading_spaces;
    let mut innermost_marker_is_quote = false;

    loop {
        if let Some((_, list_content_start)) = parse_list_marker(source, content_start)
            && list_content_start <= line_end
        {
            innermost_marker_is_quote = false;
            content_start = list_content_start;
            continue;
        }

        if bytes.get(content_start) == Some(&b'>') {
            innermost_marker_is_quote = true;
            content_start += 1;
            if matches!(bytes.get(content_start), Some(b' ' | b'\t')) {
                content_start += 1;
            }
            continue;
        }

        break;
    }

    innermost_marker_is_quote.then_some((line_start, content_start, line_end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset_classify_parse_count() {
        CLASSIFY_PARSE_COUNT.with(|parse_count| parse_count.set(0));
    }

    fn classify_parse_count() -> usize {
        CLASSIFY_PARSE_COUNT.with(Cell::get)
    }

    fn apply_augmentation_at(
        source: &str,
        current_byte: usize,
        augmentation: &EditAugmentation,
    ) -> String {
        let replacement_range =
            augmentation.replace_range.clone().unwrap_or(current_byte..current_byte);
        let mut edited_source = source.to_owned();
        edited_source
            .replace_range(replacement_range, augmentation.insert_text.as_deref().unwrap_or(""));
        edited_source
    }

    fn apply_backspace_with_default(source: &str, current_byte: usize) -> (String, usize) {
        if let Some(augmentation) = augment_edit(source, current_byte, AugmentKind::Backspace) {
            return (
                apply_augmentation_at(source, current_byte, &augmentation),
                augmentation.cursor_byte_after,
            );
        }

        let (grapheme_start, _) =
            UnicodeSegmentation::grapheme_indices(&source[..current_byte], true)
                .next_back()
                .expect("Backspace fixture must have a preceding grapheme");
        let mut edited_source = source.to_owned();
        edited_source.replace_range(grapheme_start..current_byte, "");
        (edited_source, grapheme_start)
    }

    fn assert_empty_paragraph_text_roundtrip(
        source: &str,
        current_byte: usize,
        expected_source: &str,
        expected_cursor: usize,
    ) {
        let insertion =
            augment_edit(source, current_byte, AugmentKind::InsertText(String::from("新")))
                .expect("typing in an empty paragraph must preserve its sibling slots");
        let typed_source = apply_augmentation_at(source, current_byte, &insertion);

        assert_eq!(typed_source, expected_source, "insertion mismatch for {source:?}");
        assert_eq!(insertion.cursor_byte_after, expected_cursor);

        let (restored_source, restored_cursor) =
            apply_backspace_with_default(&typed_source, insertion.cursor_byte_after);
        assert_eq!(restored_source, source, "deletion mismatch for {source:?}");
        assert_eq!(restored_cursor, current_byte);
    }

    fn editable_paragraph_slots(source: &str) -> Vec<(Vec<usize>, usize)> {
        let parsed = crate::parser::parse_markdown(source);
        let document = crate::builder::MarkdownDoc::build_structure(&parsed);
        let paragraphs =
            crate::builder::EditableParagraphMap::from_blocks(&document.blocks, source);
        paragraphs
            .runs()
            .iter()
            .flat_map(|run| {
                run.lines[run.hidden_separator_count..]
                    .iter()
                    .map(|line| (run.owner_path.clone(), line.source_byte))
            })
            .collect()
    }

    fn assert_empty_paragraph_enter_backspace_sequence(
        source: &str,
        current_byte: usize,
        expected_first_enter: &str,
        expected_second_enter: &str,
        expected_owner_path: &[usize],
    ) {
        let initial_slots = editable_paragraph_slots(source);
        assert_eq!(initial_slots, [(expected_owner_path.to_vec(), current_byte)]);

        let first_enter = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter on a real empty paragraph must add one sibling slot");
        let after_first = apply_augmentation_at(source, current_byte, &first_enter);
        let first_slots = editable_paragraph_slots(&after_first);
        assert_eq!(after_first, expected_first_enter);
        assert_eq!(first_slots.len(), 2);
        assert_eq!(first_enter.cursor_byte_after, first_slots[1].1);
        assert!(first_slots.iter().all(|(owner, _)| owner == expected_owner_path));

        let second_enter =
            augment_edit(&after_first, first_enter.cursor_byte_after, AugmentKind::Enter)
                .expect("repeated Enter must add one more sibling slot");
        let after_second =
            apply_augmentation_at(&after_first, first_enter.cursor_byte_after, &second_enter);
        let second_slots = editable_paragraph_slots(&after_second);
        assert_eq!(after_second, expected_second_enter);
        assert_eq!(second_slots.len(), 3);
        assert_eq!(second_enter.cursor_byte_after, second_slots[2].1);
        assert!(second_slots.iter().all(|(owner, _)| owner == expected_owner_path));

        let (restored_first, restored_first_cursor) =
            apply_backspace_with_default(&after_second, second_enter.cursor_byte_after);
        assert_eq!(
            (restored_first, restored_first_cursor),
            (after_first.clone(), first_enter.cursor_byte_after)
        );
        let (restored_source, restored_cursor) =
            apply_backspace_with_default(&after_first, first_enter.cursor_byte_after);
        assert_eq!((restored_source, restored_cursor), (source.to_owned(), current_byte));
    }

    #[test]
    fn regular_text_insertion_skips_document_classification_parse() {
        reset_classify_parse_count();
        let source = "first paragraph\n\nsecond paragraph";

        let augmentation =
            augment_edit(source, "first".len(), AugmentKind::InsertText(String::from("x")));

        assert!(augmentation.is_none());
        assert_eq!(classify_parse_count(), 0);
    }

    #[test]
    fn padded_top_level_empty_paragraph_enter_backspace_preserves_source_and_slots() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("head{newline} {newline} {newline}tail");
            let current_byte = format!("head{newline} {newline}").len();
            let after_first = format!("head{newline} {newline} {newline}{newline}tail");
            let after_second = format!("head{newline} {newline} {newline}{newline}{newline}tail");

            assert_empty_paragraph_enter_backspace_sequence(
                &source,
                current_byte,
                &after_first,
                &after_second,
                &[],
            );
        }
    }

    #[test]
    fn trailing_single_empty_paragraph_enter_accounts_for_new_hidden_separator() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("head{newline}");
            let current_byte = source.len();
            let initial_slots = editable_paragraph_slots(&source);
            assert_eq!(initial_slots, [(Vec::new(), current_byte)]);

            let enter = augment_edit(&source, current_byte, AugmentKind::Enter)
                .expect("Enter must add a trailing empty paragraph slot");
            let edited_source = apply_augmentation_at(&source, current_byte, &enter);
            let expected_source = format!("head{newline}{newline}{newline}");
            let entered_slots = editable_paragraph_slots(&edited_source);
            assert_eq!(edited_source, expected_source);
            assert_eq!(entered_slots.len(), initial_slots.len() + 1);
            assert_eq!(enter.cursor_byte_after, entered_slots[1].1);

            let (after_backspace, backspace_cursor) =
                apply_backspace_with_default(&edited_source, enter.cursor_byte_after);
            let remaining_slots = editable_paragraph_slots(&after_backspace);
            assert_eq!(remaining_slots.len(), initial_slots.len());
            assert_eq!(backspace_cursor, remaining_slots[0].1);
        }
    }

    #[test]
    fn blockquote_empty_paragraph_enter_backspace_stays_in_owner() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("> first{newline}>{newline}>{newline}> second");
            let current_byte = format!("> first{newline}>{newline}>").len();
            let after_first = format!("> first{newline}>{newline}>{newline}>{newline}> second");
            let after_second =
                format!("> first{newline}>{newline}>{newline}>{newline}>{newline}> second");

            assert_empty_paragraph_enter_backspace_sequence(
                &source,
                current_byte,
                &after_first,
                &after_second,
                &[0],
            );
        }
    }

    #[test]
    fn all_empty_blockquote_with_multiple_slots_keeps_navigating_in_its_owner() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!(">{newline}>{newline}>");
            let current_byte = source.len();
            let initial_slots = editable_paragraph_slots(&source);
            assert_eq!(initial_slots.len(), 3);
            assert!(initial_slots.iter().all(|(owner, _)| owner == &[0]));

            let enter = augment_edit(&source, current_byte, AugmentKind::Enter)
                .expect("a multi-slot empty quote must keep Enter inside its owner");
            let edited_source = apply_augmentation_at(&source, current_byte, &enter);
            let expected_source = format!(">{newline}>{newline}>{newline}>");
            let entered_slots = editable_paragraph_slots(&edited_source);
            assert_eq!(edited_source, expected_source);
            assert_eq!(entered_slots.len(), initial_slots.len() + 1);
            assert_eq!(enter.cursor_byte_after, entered_slots[3].1);

            let (restored_source, restored_cursor) =
                apply_backspace_with_default(&edited_source, enter.cursor_byte_after);
            assert_eq!((restored_source, restored_cursor), (source, current_byte));
        }
    }

    #[test]
    fn list_empty_paragraph_enter_backspace_stays_in_owner() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("- first{newline}{newline}{newline}  second");
            let current_byte = format!("- first{newline}{newline}").len();
            let after_first = format!("- first{newline}{newline}{newline}{newline}  second");
            let after_second =
                format!("- first{newline}{newline}{newline}{newline}{newline}  second");

            assert_empty_paragraph_enter_backspace_sequence(
                &source,
                current_byte,
                &after_first,
                &after_second,
                &[0],
            );
        }
    }

    #[test]
    fn regular_mid_line_backspace_skips_document_classification_parse() {
        reset_classify_parse_count();
        let source = "first paragraph\n\nsecond paragraph";

        let augmentation = augment_edit(source, "first".len(), AugmentKind::Backspace);

        assert!(augmentation.is_none());
        assert_eq!(classify_parse_count(), 0);
    }

    #[test]
    fn classification_guards_allow_special_edit_candidates() {
        reset_classify_parse_count();
        let separator_source = "first\n\nsecond";
        let separator_byte = "first\n".len();

        let insertion = augment_edit(
            separator_source,
            separator_byte,
            AugmentKind::InsertText(String::from("x")),
        );

        assert!(insertion.is_some());
        assert_eq!(classify_parse_count(), 1);

        reset_classify_parse_count();
        let interblock_source = "first\n\n中\n\nsecond";
        let paragraph_end = "first\n\n中".len();

        let backspace = augment_edit(interblock_source, paragraph_end, AugmentKind::Backspace);

        assert!(backspace.is_some());
        assert_eq!(classify_parse_count(), 1);
    }

    #[test]
    fn parses_supported_list_marker_boundaries() {
        assert_eq!(parse_list_marker("42. item", 0), Some((ListBullet::Ordered(42), 4)));
        assert_eq!(parse_list_marker("-\titem", 0), Some((ListBullet::Bullet, 2)));
        assert_eq!(parse_list_marker("-   item", 0), Some((ListBullet::Bullet, 4)));
        assert_eq!(parse_list_marker("10.\titem", 0), Some((ListBullet::Ordered(10), 4)));
        assert_eq!(parse_list_marker("- [X] done", 0), Some((ListBullet::TaskList(true), 6)));
        assert_eq!(parse_list_marker("-   [ ]\titem", 0), Some((ListBullet::TaskList(false), 8)));
        assert_eq!(parse_list_marker("+ item", 0), Some((ListBullet::Bullet, 2)));
        assert_eq!(parse_list_marker("* item", 0), Some((ListBullet::Bullet, 2)));
        assert_eq!(parse_list_marker("7)\titem", 0), Some((ListBullet::Ordered(7), 3)));
        assert_eq!(parse_list_marker("- [ ] item", 0), Some((ListBullet::TaskList(false), 6)));
        assert_eq!(parse_list_marker("1234567890. item", 0), None);
    }

    #[test]
    fn list_enter_normalizes_task_markers_without_a_content_separator() {
        let cases = ["- [ ]todo", "- [x]done", "- [X]done"];

        for source in cases {
            let augmentation = augment_edit(source, source.len(), AugmentKind::Enter)
                .expect("Enter should continue a task list item");

            assert_eq!(
                augmentation.insert_text.as_deref(),
                Some("\n- [ ] "),
                "continued task marker must remain valid for {source:?}"
            );
        }
    }

    #[test]
    fn paragraph_enter_before_soft_break_preserves_following_source_line() {
        let source = "first line\nsecond line";
        let current_byte = "first line".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("paragraph Enter should split the paragraph");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(augmentation.insert_text.as_deref(), Some("\n\n"));
        assert_eq!(augmentation.cursor_byte_after, current_byte + 2);
        assert_eq!(edited_source, "first line\n\n\nsecond line");
    }

    #[test]
    fn paragraph_enter_before_crlf_soft_break_preserves_crlf_line_endings() {
        let source = "first line\r\nsecond line";
        let current_byte = "first line".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("paragraph Enter should split before a CRLF soft break");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(augmentation.insert_text.as_deref(), Some("\r\n\r\n"));
        assert_eq!(augmentation.cursor_byte_after, current_byte + 4);
        assert_eq!(edited_source, "first line\r\n\r\n\r\nsecond line");
    }

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
    fn enter_at_odd_backslash_hard_break_preserves_escaped_backslashes() {
        let source = "first\\\\\\\nsecond";
        let current_byte = "first".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter should consume only the final unescaped backslash");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "first\\\\\n\nsecond");
        assert_eq!(augmentation.cursor_byte_after, "first\\\\\n\n".len());
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
    fn enter_before_even_backslashes_keeps_soft_break_semantics() {
        let source = "first\\\\\nsecond";
        let current_byte = "first".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter before escaped backslashes should use paragraph splitting");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "first\n\n\\\\\nsecond");
    }

    #[test]
    fn heading_enter_before_single_newline_creates_editable_empty_paragraph() {
        let source = "# Heading\nparagraph";
        let current_byte = "# Heading".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("heading-end Enter should create an empty paragraph");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(augmentation.insert_text.as_deref(), Some("\n\n"));
        assert_eq!(augmentation.cursor_byte_after, current_byte + 2);
        assert_eq!(edited_source, "# Heading\n\n\nparagraph");
    }

    #[test]
    fn backspace_at_start_of_space_split_paragraph_restores_original_space() {
        let original_source = "hello world";
        let split_byte = "hello ".len();
        let enter_augmentation = augment_edit(original_source, split_byte, AugmentKind::Enter)
            .expect("paragraph Enter should split at the cursor");
        let split_source = apply_augmentation_at(original_source, split_byte, &enter_augmentation);

        let backspace_augmentation = augment_edit(
            &split_source,
            enter_augmentation.cursor_byte_after,
            AugmentKind::Backspace,
        )
        .expect("Backspace at the second paragraph start should join both paragraphs");
        let restored_source = apply_augmentation_at(
            &split_source,
            enter_augmentation.cursor_byte_after,
            &backspace_augmentation,
        );

        assert_eq!(restored_source, original_source);
        assert_eq!(backspace_augmentation.cursor_byte_after, split_byte);
    }

    #[test]
    fn backspace_at_paragraph_start_after_heading_joins_heading() {
        let source = "# Heading\n\nparagraph";
        let current_byte = "# Heading\n\n".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace at a block start should remove the complete source boundary");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "# Headingparagraph");
        assert_eq!(augmentation.cursor_byte_after, "# Heading".len());
    }

    #[test]
    fn backspace_at_atx_heading_marker_start_is_consumed() {
        let source = "previous\n# Heading";
        let marker_start = "previous\n".len();

        let augmentation = augment_edit(source, marker_start, AugmentKind::Backspace)
            .expect("heading marker start must guard the preceding source boundary");

        assert_eq!(augmentation.insert_text.as_deref(), Some(""));
        assert_eq!(augmentation.replace_range, None);
        assert_eq!(augmentation.cursor_byte_after, marker_start);
    }

    #[test]
    fn enter_then_backspace_restores_plain_paragraph_and_heading_cases() {
        let cases = [
            ("hello", "hello".len()),
            ("# hello world", "# he".len()),
            ("# Heading", "# Heading".len()),
            ("first\n\nsecond", "first".len()),
            ("# Heading\n\nparagraph", "# Heading".len()),
        ];

        for (source, current_byte) in cases {
            let enter_augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
                .unwrap_or_else(|| panic!("Enter must handle fixture {source:?}"));
            let edited_source = apply_augmentation_at(source, current_byte, &enter_augmentation);
            let backspace_augmentation = augment_edit(
                &edited_source,
                enter_augmentation.cursor_byte_after,
                AugmentKind::Backspace,
            )
            .unwrap_or_else(|| panic!("Backspace must reverse Enter for {source:?}"));
            let restored_source = apply_augmentation_at(
                &edited_source,
                enter_augmentation.cursor_byte_after,
                &backspace_augmentation,
            );

            assert_eq!(restored_source, source, "Enter/Backspace mismatch for {source:?}");
            assert_eq!(backspace_augmentation.cursor_byte_after, current_byte);
        }
    }

    #[test]
    fn backspace_after_repeated_trailing_enter_removes_only_latest_empty_line() {
        let source = "first";
        let first_enter = augment_edit(source, source.len(), AugmentKind::Enter)
            .expect("first Enter should create a trailing paragraph");
        let after_first_enter = apply_augmentation_at(source, source.len(), &first_enter);
        let second_enter =
            augment_edit(&after_first_enter, first_enter.cursor_byte_after, AugmentKind::Enter)
                .expect("second Enter should add one trailing empty line");
        let after_second_enter =
            apply_augmentation_at(&after_first_enter, first_enter.cursor_byte_after, &second_enter);

        let backspace = augment_edit(
            &after_second_enter,
            second_enter.cursor_byte_after,
            AugmentKind::Backspace,
        )
        .expect("Backspace should remove the latest trailing empty line");
        let restored =
            apply_augmentation_at(&after_second_enter, second_enter.cursor_byte_after, &backspace);

        assert_eq!(restored, after_first_enter);
        assert_eq!(backspace.cursor_byte_after, first_enter.cursor_byte_after);
    }

    #[test]
    fn backspace_between_blocks_reverses_three_enters_one_at_a_time() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let initial_source = format!("前段{newline}{newline}后段");
            let initial_cursor = "前段".len();
            let mut edit_states = vec![(initial_source, initial_cursor)];

            for _ in 0..3 {
                let (source, cursor) =
                    edit_states.last().expect("the initial edit state must exist");
                let enter = augment_edit(source, *cursor, AugmentKind::Enter)
                    .expect("Enter between blocks must create an editable empty paragraph");
                edit_states.push((
                    apply_augmentation_at(source, *cursor, &enter),
                    enter.cursor_byte_after,
                ));
            }

            for expected_state_index in (0..3).rev() {
                let (source, cursor) =
                    edit_states.last().expect("the latest edit state must exist");
                let backspace = augment_edit(source, *cursor, AugmentKind::Backspace)
                    .expect("Backspace must reverse the nearest Enter");
                let edited_source = apply_augmentation_at(source, *cursor, &backspace);
                let expected_state = &edit_states[expected_state_index];

                assert_eq!(edited_source, expected_state.0, "source mismatch for {newline:?}");
                assert_eq!(
                    backspace.cursor_byte_after, expected_state.1,
                    "cursor mismatch for {newline:?}"
                );
                edit_states.pop();
            }
        }
    }

    #[test]
    fn backspace_in_blank_run_before_atomic_block_keeps_nearest_empty_line() {
        let cases = [
            ("前段\n\n\n\n---", "前段\n\n\n".len(), "前段\n\n\n---", "前段\n\n".len()),
            (
                "# 标题\r\n\r\n\r\n\r\n```",
                "# 标题\r\n\r\n\r\n".len(),
                "# 标题\r\n\r\n\r\n```",
                "# 标题\r\n\r\n".len(),
            ),
        ];

        for (source, current_byte, expected_source, expected_cursor) in cases {
            let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
                .expect("Backspace in the blank run must remove one empty line");
            let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

            assert_eq!(edited_source, expected_source);
            assert_eq!(augmentation.cursor_byte_after, expected_cursor);
        }
    }

    #[test]
    fn backspace_on_fenced_code_blank_line_keeps_default_physical_newline_edit() {
        let source = "```\nfirst\n\n\nsecond\n```";
        let current_byte = "```\nfirst\n\n".len();

        assert!(matches!(classify_enter_context(source, current_byte), EnterContext::CodeBlock));
        assert!(
            augment_edit(source, current_byte, AugmentKind::Backspace).is_none(),
            "Backspace inside fenced code must delete the preceding physical newline by default"
        );
    }

    #[test]
    fn backspace_without_preceding_block_removes_only_one_empty_slot() {
        let cases = [
            ("\n\n", 2, "\n", 1),
            ("\n\n\nhead", 2, "\n\nhead", 1),
            ("\r\n\r\n", 4, "\r\n", 2),
            ("\r\n\r\n\r\nhead", 4, "\r\n\r\nhead", 2),
        ];

        for (source, current_byte, expected_source, expected_cursor) in cases {
            let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
                .expect("Backspace must remove only the nearest leading empty slot");
            let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

            assert_eq!(edited_source, expected_source, "source mismatch for {source:?}");
            assert_eq!(augmentation.cursor_byte_after, expected_cursor);
        }
    }

    #[test]
    fn list_enter_preserves_marker_style_and_resets_completed_tasks() {
        let cases = [
            ("* item", "\n* "),
            ("+ item", "\n+ "),
            ("7) item", "\n8) "),
            ("-\titem", "\n-\t"),
            ("- [x] done", "\n- [ ] "),
            ("- [ ]\ttodo", "\n- [ ]\t"),
        ];

        for (source, expected_insert) in cases {
            let augmentation = augment_edit(source, source.len(), AugmentKind::Enter)
                .unwrap_or_else(|| panic!("Enter should continue list item {source:?}"));

            assert_eq!(
                augmentation.insert_text.as_deref(),
                Some(expected_insert),
                "continuation marker mismatch for {source:?}"
            );
        }
    }

    #[test]
    fn backspace_removes_markers_that_use_whitespace_separators() {
        for source in ["-\t", ">\t", "1.\t", "1.  ", "1. \t"] {
            let augmentation = augment_edit(source, source.len(), AugmentKind::Backspace)
                .unwrap_or_else(|| panic!("Backspace should remove marker {source:?}"));

            assert_eq!(augmentation.replace_range, Some(0..source.len()));
            assert_eq!(augmentation.cursor_byte_after, 0);
        }
    }

    #[test]
    fn backspace_at_soft_line_start_removes_single_source_newline() {
        let source = "first line\nsecond line";
        let current_byte = "first line\n".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace at a soft line start should join both visual lines");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "first linesecond line");
        assert_eq!(augmentation.replace_range, Some("first line".len()..current_byte));
        assert_eq!(augmentation.cursor_byte_after, "first line".len());
    }

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
    fn backspace_after_crlf_hard_break_removes_the_complete_boundary() {
        let source = "first  \r\nsecond";
        let current_byte = "first  \r\n".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace after a CRLF hard break should join both visual lines");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "firstsecond");
        assert_eq!(augmentation.cursor_byte_after, "first".len());
    }

    #[test]
    fn backspace_after_odd_backslash_hard_break_preserves_escaped_backslashes() {
        let source = "first\\\\\\\nsecond";
        let current_byte = "first\\\\\\\n".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace should remove only the final unescaped backslash");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "first\\\\second");
        assert_eq!(augmentation.cursor_byte_after, "first\\\\".len());
    }

    #[test]
    fn backspace_keeps_non_hard_break_trailing_characters() {
        let cases = [("first \nsecond", "first second"), ("first\\\\\nsecond", "first\\\\second")];

        for (source, expected_source) in cases {
            let current_byte = source.find("second").expect("fixture must contain second");
            let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
                .expect("Backspace at a soft line start should join both visual lines");
            let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

            assert_eq!(edited_source, expected_source);
        }
    }

    #[test]
    fn backspace_at_crlf_paragraph_start_removes_complete_boundary() {
        let source = "first\r\n\r\nsecond";
        let current_byte = "first\r\n\r\n".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace should treat CRLF sequences as complete source newlines");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "firstsecond");
        assert_eq!(augmentation.replace_range, Some("first".len()..current_byte));
        assert_eq!(augmentation.cursor_byte_after, "first".len());
    }

    #[test]
    fn backspace_on_crlf_empty_line_removes_complete_newline_sequence() {
        let source = "first\r\n\r\nsecond";
        let current_byte = "first\r\n".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace on a CRLF empty line should use the empty-line edit");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "first\r\nsecond");
        assert_eq!(augmentation.replace_range, Some(current_byte..current_byte + 2));
        assert_eq!(augmentation.cursor_byte_after, "first".len());
    }

    #[test]
    fn insert_text_in_crlf_empty_separator_preserves_block_boundaries() {
        let source = "first\r\n\r\nsecond";
        let current_byte = "first\r\n".len();

        let augmentation =
            augment_edit(source, current_byte, AugmentKind::InsertText(String::from("中")))
                .expect("typing on a CRLF separator line should create an editable paragraph");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "first\r\n\r\n中\r\n\r\nsecond");
        assert_eq!(augmentation.cursor_byte_after, "first\r\n\r\n中".len());
    }

    #[test]
    fn insert_text_in_leading_empty_lines_preserves_every_slot_and_is_reversible() {
        let cases = [
            ("\n\nhead", 0, "新\n\n\nhead", "新".len()),
            ("\n\nhead", 1, "\n新\n\nhead", "\n新".len()),
            ("\r\n\r\nhead", 0, "新\r\n\r\n\r\nhead", "新".len()),
            ("\r\n\r\nhead", 2, "\r\n新\r\n\r\nhead", "\r\n新".len()),
        ];

        for (source, current_byte, expected_source, expected_cursor) in cases {
            assert_empty_paragraph_text_roundtrip(
                source,
                current_byte,
                expected_source,
                expected_cursor,
            );
        }
    }

    #[test]
    fn insert_text_in_each_pure_empty_document_line_preserves_all_lines() {
        let cases = [
            ("\n\n", 0, "新\n\n\n", "新".len()),
            ("\n\n", 1, "\n新\n\n", "\n新".len()),
            ("\n\n", 2, "\n\n新", "\n\n新".len()),
            ("\r\n\r\n", 0, "新\r\n\r\n\r\n", "新".len()),
            ("\r\n\r\n", 2, "\r\n新\r\n\r\n", "\r\n新".len()),
            ("\r\n\r\n", 4, "\r\n\r\n新", "\r\n\r\n新".len()),
        ];

        for (source, current_byte, expected_source, expected_cursor) in cases {
            assert_empty_paragraph_text_roundtrip(
                source,
                current_byte,
                expected_source,
                expected_cursor,
            );
        }
    }

    #[test]
    fn insert_text_in_each_interblock_empty_slot_preserves_the_complete_run() {
        let cases = [
            ("head\n\n\n\nfoot", "head\n\n".len(), "head\n\n新\n\n\nfoot"),
            ("head\n\n\n\nfoot", "head\n\n\n".len(), "head\n\n\n新\n\nfoot"),
            ("head\r\n\r\n\r\n\r\nfoot", "head\r\n\r\n".len(), "head\r\n\r\n新\r\n\r\n\r\nfoot"),
            (
                "head\r\n\r\n\r\n\r\nfoot",
                "head\r\n\r\n\r\n".len(),
                "head\r\n\r\n\r\n新\r\n\r\nfoot",
            ),
        ];

        for (source, current_byte, expected_source) in cases {
            assert_empty_paragraph_text_roundtrip(
                source,
                current_byte,
                expected_source,
                current_byte + "新".len(),
            );
        }
    }

    #[test]
    fn insert_text_in_first_trailing_empty_line_preserves_every_slot_and_is_reversible() {
        let cases = [
            ("head\n\n\n\n", "head\n\n".len(), "head\n\n新\n\n\n"),
            ("head\r\n\r\n\r\n\r\n", "head\r\n\r\n".len(), "head\r\n\r\n新\r\n\r\n\r\n"),
        ];

        for (source, current_byte, expected_source) in cases {
            assert_empty_paragraph_text_roundtrip(
                source,
                current_byte,
                expected_source,
                current_byte + "新".len(),
            );
        }
    }

    #[test]
    fn insert_text_with_source_newline_keeps_default_semantics_on_empty_lines() {
        for text in ["\n", "\r\n", "新\n行"] {
            assert!(
                augment_edit("\n\nhead", 0, AugmentKind::InsertText(String::from(text))).is_none(),
                "newline insertion must not use empty-paragraph normalization: {text:?}"
            );
        }
    }

    #[test]
    fn list_enter_in_crlf_document_continues_with_crlf() {
        let source = "- first\r\n- second";
        let current_byte = "- first".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("list Enter should continue the current CRLF list item");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(augmentation.insert_text.as_deref(), Some("\r\n- "));
        assert_eq!(edited_source, "- first\r\n- \r\n- second");
    }

    #[test]
    fn empty_list_enter_in_crlf_document_preserves_following_newline() {
        let source = "- \r\nnext";
        let current_byte = "- ".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter should exit an empty CRLF list item");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "\r\nnext");
        assert_eq!(augmentation.cursor_byte_after, 0);
    }

    #[test]
    fn empty_blockquote_enter_in_crlf_document_exits_quote() {
        let source = "> \r\nnext";
        let current_byte = "> ".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter should exit an empty CRLF blockquote line");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "\r\nnext");
        assert_eq!(augmentation.cursor_byte_after, 0);
    }

    #[test]
    fn indented_blockquote_enter_preserves_indent_and_marker() {
        let source = "  > quote";
        let current_byte = source.len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter should continue a blockquote indented by up to three spaces");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(augmentation.insert_text.as_deref(), Some("\n  > "));
        assert_eq!(edited_source, "  > quote\n  > ");
    }

    #[test]
    fn nested_blockquote_enter_preserves_complete_marker_prefix() {
        let source = "> > quote";
        let current_byte = source.len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter should continue every active blockquote level");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(augmentation.insert_text.as_deref(), Some("\n> > "));
        assert_eq!(edited_source, "> > quote\n> > ");
    }

    #[test]
    fn backspace_after_non_text_block_uses_existing_specialized_or_fallback_behavior() {
        // 列表边界已由显式并入测试覆盖（M4），此处只保留仍回退默认计划的叶块。
        let cases = ["---\n\nparagraph", "```\ncode\n```\n\nparagraph"];

        for source in cases {
            let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");

            assert!(
                augment_edit(source, current_byte, AugmentKind::Backspace).is_none(),
                "generic paragraph joining must not consume a {source:?} boundary"
            );
        }
    }

    #[test]
    fn pulldown_cmark_emits_heading_event_for_setext_heading() {
        use pulldown_cmark::{Event, Parser, Tag};

        let source = "Title\n===\n";
        let heading_ranges: Vec<std::ops::Range<usize>> =
            Parser::new_ext(source, pulldown_cmark::Options::all())
                .into_offset_iter()
                .filter_map(|(event, range)| {
                    matches!(event, Event::Start(Tag::Heading { .. })).then_some(range)
                })
                .collect();

        assert_eq!(heading_ranges.len(), 1, "setext heading must surface as a heading event");
        let heading_range = &heading_ranges[0];
        assert!(
            !source[heading_range.start..].starts_with('#'),
            "setext heading source does not start with an ATX marker: {:?}",
            &source[heading_range.clone()]
        );
    }

    #[test]
    fn setext_heading_is_classified_as_setext_not_atx() {
        let source = "Title\n===";
        let underline_end = source.len();

        assert!(
            matches!(
                classify_enter_context(source, "Title".len()),
                EnterContext::SetextHeading { underline_end: end, .. } if end == underline_end
            ),
            "setext title text must not use ATX heading semantics"
        );
        assert!(
            matches!(
                classify_enter_context(source, source.len()),
                EnterContext::SetextHeading { underline_end: end, .. } if end == underline_end
            ),
            "setext underline must not use ATX heading semantics"
        );
    }

    #[test]
    fn setext_heading_enter_creates_editable_paragraph_after_the_underline() {
        let source = "Title\n=====\npara";
        let current_byte = "Title".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter inside a setext heading must preserve the heading source");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "Title\n=====\n\n\npara");
        assert_eq!(augmentation.cursor_byte_after, "Title\n=====\n\n".len());
    }

    #[test]
    fn setext_heading_enter_from_the_underline_line_appends_after_it() {
        let source = "Title\n-----\npara";
        let current_byte = "Title\n-----".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter on a setext underline must preserve the heading source");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "Title\n-----\n\n\npara");
        assert_eq!(augmentation.cursor_byte_after, "Title\n-----\n\n".len());
    }

    #[test]
    fn setext_heading_enter_at_document_end_preserves_existing_empty_paragraph() {
        let source = "Title\n=====\n";
        let current_byte = "Title".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter inside a trailing setext heading must preserve the heading source");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "Title\n=====\n\n\n");
        assert_eq!(augmentation.cursor_byte_after, "Title\n=====\n\n".len());
    }

    #[test]
    fn definition_list_is_classified_the_way_the_renderer_lays_it_out() {
        let source = "term\n: definition";

        assert!(
            matches!(
                classify_enter_context(source, source.len()),
                EnterContext::TopLevelParagraphEnd
            ),
            "editing classifier and renderer must share parser options"
        );
    }

    #[test]
    fn indented_code_block_enter_continues_the_current_line_indent() {
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
    fn indented_code_block_blank_line_inherits_nearest_code_indent() {
        let source = "    first\n\n    second";
        let current_byte = "    first\n".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter on an internal blank code line should inherit code indentation");
        let after_enter = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(augmentation.insert_text.as_deref(), Some("\n    "));
        assert!(
            augment_edit(
                &after_enter,
                augmentation.cursor_byte_after,
                AugmentKind::InsertText(String::from("y")),
            )
            .is_none(),
            "typing after Enter on a blank code line must stay in the code block"
        );
    }

    #[test]
    fn fenced_code_block_enter_still_uses_the_default_plan() {
        let source = "```\nlet x = 1;\n```";
        let current_byte = "```\nlet x = 1;".len();

        assert!(
            augment_edit(source, current_byte, AugmentKind::Enter).is_none(),
            "fenced code blocks keep using the default single-newline plan"
        );
    }

    #[test]
    fn table_enter_moves_to_the_next_cell_content_start() {
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
            source.rfind('|').expect("fixture must contain a closing pipe")
        );
    }

    #[test]
    fn backspace_merging_paragraph_into_unmergeable_leaf_block_is_noop() {
        let cases = [
            "```\ncode\n```\nparagraph",
            "    code\nparagraph",
            "---\nparagraph",
            "Title\n===\nparagraph",
        ];

        for source in cases {
            let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");
            let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
                .unwrap_or_else(|| {
                    panic!("leaf-block boundary in {source:?} must be guarded, not fall back")
                });
            let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

            assert_eq!(edited_source, source, "guarded backspace must not change {source:?}");
            assert_eq!(
                augmentation.cursor_byte_after, current_byte,
                "guarded backspace must keep the cursor in {source:?}"
            );
        }
    }

    #[test]
    fn backspace_guard_for_leaf_block_boundary_preserves_crlf_source() {
        let source = "```\r\ncode\r\n```\r\nparagraph";
        let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");

        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("CRLF leaf-block boundary must be guarded");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, source);
        assert_eq!(augmentation.cursor_byte_after, current_byte);
    }

    #[test]
    fn backspace_across_blank_line_before_leaf_block_keeps_default_fallback() {
        let cases = ["```\ncode\n```\n\nparagraph", "    code\n\nparagraph", "---\n\nparagraph"];

        for source in cases {
            let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");

            assert!(
                augment_edit(source, current_byte, AugmentKind::Backspace).is_none(),
                "removing one newline of a blank separator before a leaf block is safe: {source:?}"
            );
        }
    }

    #[test]
    fn backspace_at_paragraph_start_after_blockquote_merges_with_explicit_marker() {
        let cases = [
            ("> quote\n\nparagraph", "> quote\n> paragraph", "> quote\n> "),
            ("> > quote\n\nparagraph", "> > quote\n> > paragraph", "> > quote\n> > "),
            ("  > quote\n\nparagraph", "  > quote\n  > paragraph", "  > quote\n  > "),
        ];

        for (source, expected_source, expected_cursor_prefix) in cases {
            let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");
            let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
                .unwrap_or_else(|| {
                    panic!("blockquote boundary in {source:?} must merge with an explicit marker")
                });
            let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

            assert_eq!(edited_source, expected_source, "merge mismatch for {source:?}");
            assert_eq!(
                augmentation.cursor_byte_after,
                expected_cursor_prefix.len(),
                "cursor must land before the merged paragraph content for {source:?}"
            );
        }
    }

    #[test]
    fn backspace_at_paragraph_start_after_list_item_merges_as_continuation_line() {
        let cases = [
            ("- item\n\nparagraph", "- item\n  paragraph", "- item\n  "),
            ("1. item\n\nparagraph", "1. item\n   paragraph", "1. item\n   "),
            ("- [x] done\n\nparagraph", "- [x] done\n      paragraph", "- [x] done\n      "),
            (
                "- outer\n  - inner\n\nparagraph",
                "- outer\n  - inner\n    paragraph",
                "- outer\n  - inner\n    ",
            ),
        ];

        for (source, expected_source, expected_cursor_prefix) in cases {
            let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");
            let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
                .unwrap_or_else(|| {
                    panic!("list boundary in {source:?} must merge as a continuation line")
                });
            let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

            assert_eq!(edited_source, expected_source, "merge mismatch for {source:?}");
            assert_eq!(
                augmentation.cursor_byte_after,
                expected_cursor_prefix.len(),
                "cursor must land at the content column for {source:?}"
            );
        }
    }

    #[test]
    fn backspace_merging_into_blockquote_or_list_preserves_crlf_line_endings() {
        let cases = [
            ("> quote\r\n\r\nparagraph", "> quote\r\n> paragraph", "> quote\r\n> "),
            ("- item\r\n\r\nparagraph", "- item\r\n  paragraph", "- item\r\n  "),
        ];

        for (source, expected_source, expected_cursor_prefix) in cases {
            let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");
            let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
                .unwrap_or_else(|| panic!("CRLF boundary in {source:?} must merge explicitly"));
            let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

            assert_eq!(edited_source, expected_source, "merge mismatch for {source:?}");
            assert_eq!(augmentation.cursor_byte_after, expected_cursor_prefix.len());
        }
    }

    #[test]
    fn deleting_last_interblock_paragraph_grapheme_restores_one_editable_blank_line() {
        let source = "first\n\n中\n\nsecond";
        let cursor_byte = "first\n\n中".len();

        let augmentation = augment_edit(source, cursor_byte, AugmentKind::Backspace).expect(
            "deleting the final paragraph grapheme should preserve one editable blank line",
        );
        let replacement =
            augmentation.replace_range.expect("final grapheme deletion needs a range");
        let mut result = source.to_string();
        result.replace_range(replacement, augmentation.insert_text.as_deref().unwrap_or(""));

        assert_eq!(result, "first\n\n\nsecond");
        assert_eq!(augmentation.cursor_byte_after, "first\n\n".len());
    }

    #[test]
    fn deleting_last_interblock_paragraph_grapheme_preserves_crlf_boundaries() {
        let source = "first\r\n\r\n中\r\n\r\nsecond";
        let cursor_byte = "first\r\n\r\n中".len();

        let augmentation = augment_edit(source, cursor_byte, AugmentKind::Backspace)
            .expect("CRLF paragraphs should normalize the final grapheme deletion");
        let replacement =
            augmentation.replace_range.expect("final grapheme deletion needs a range");
        let mut result = source.to_string();
        result.replace_range(replacement, augmentation.insert_text.as_deref().unwrap_or(""));

        assert_eq!(result, "first\r\n\r\n\r\nsecond");
        assert_eq!(augmentation.cursor_byte_after, "first\r\n\r\n".len());
    }

    #[test]
    fn enter_on_blank_line_between_loose_list_items_inserts_plain_newline() {
        // 松散列表 item 的 range 含块间空行；命中测试用去尾换行的 content_end，
        // 因此空行不再属于前一 item，Enter 只加宽块间距而非续 `- ` marker。
        let source = "- a\n\n- b";
        let current_byte = "- a\n".len();

        assert!(
            matches!(
                classify_enter_context(source, current_byte),
                EnterContext::EmptyBlockSeparatorLine
            ),
            "blank separator line must not be classified as part of the preceding list item"
        );
        assert!(
            matches!(classify_enter_context(source, "- a".len()), EnterContext::ListItem { .. }),
            "the item content end must still continue the list marker"
        );

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter on a blank separator line should emit a plain newline");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(augmentation.insert_text.as_deref(), Some("\n"));
        assert_eq!(augmentation.replace_range, None);
        assert_eq!(augmentation.cursor_byte_after, current_byte + 1);
        assert_eq!(edited_source, "- a\n\n\n- b");
    }

    #[test]
    fn enter_on_blank_line_after_code_block_inserts_plain_newline() {
        // 代码块 range 的结尾换行统一按去尾换行归一化；其后的空行是块分隔行，
        // Enter 只加宽块间距，不落入 CodeBlock（无增强）语义。
        let source = "```\ncode\n```\n\npara";
        let current_byte = "```\ncode\n```\n".len();

        assert!(
            matches!(
                classify_enter_context(source, current_byte),
                EnterContext::EmptyBlockSeparatorLine
            ),
            "blank line after a code block must not be classified as CodeBlock"
        );

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter on a blank separator line should emit a plain newline");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(augmentation.insert_text.as_deref(), Some("\n"));
        assert_eq!(augmentation.replace_range, None);
        assert_eq!(augmentation.cursor_byte_after, current_byte + 1);
        assert_eq!(edited_source, "```\ncode\n```\n\n\npara");
    }

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
    fn list_enter_before_blank_separator_preserves_source_for_each_marker() {
        for newline in ["\n", "\r\n"] {
            for separator in ["", " ", "\t"] {
                for (first, marker) in
                    [("- first", "- "), ("3. first", "4. "), ("- [x] first", "- [ ] ")]
                {
                    let suffix = format!("{newline}{separator}{newline}  following");
                    let source = format!("{first}{suffix}");
                    let augmentation = augment_edit(&source, first.len(), AugmentKind::Enter)
                        .expect("Enter must create a new list item before the separator");
                    let edited = apply_augmentation_at(&source, first.len(), &augmentation);
                    assert_eq!(edited, format!("{first}{newline}{marker}{suffix}"));
                    assert_eq!(
                        augmentation.cursor_byte_after,
                        first.len() + newline.len() + marker.len()
                    );
                }
            }
        }
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
        assert!(edited_source.contains('\\'), "escaped backslashes must remain: {edited_source:?}");
    }

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

    #[test]
    fn backspace_does_not_strip_a_new_nested_container_after_a_hard_break() {
        for source in ["plain\\\n> quote", "- item\\\n  > quote"] {
            let current_byte = source.find("quote").expect("fixture contains quote content");
            let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace);
            let edited_source = augmentation
                .as_ref()
                .map(|augmentation| apply_augmentation_at(source, current_byte, augmentation))
                .unwrap_or_else(|| source.to_owned());

            assert!(
                edited_source.contains('>'),
                "Backspace must not erase a structurally new quote marker in {source:?}"
            );
        }
    }

    #[test]
    fn backspace_joins_a_hard_break_when_the_nested_container_path_matches() {
        let source = "- > first\\\n  > second";
        let current_byte = source.find("second").expect("fixture contains continuation content");

        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("matching nested container paths should be joinable");

        assert_eq!(apply_augmentation_at(source, current_byte, &augmentation), "- > firstsecond");
    }

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

    #[test]
    fn paragraph_enter_before_a_single_space_preserves_a_reversible_separator() {
        let source = "left right";
        let current_byte = "left".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("paragraph Enter should split at the word boundary");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "left \n\nright");
        assert_eq!(augmentation.cursor_byte_after, "left \n\n".len());
    }

    #[test]
    fn paragraph_enter_after_a_single_space_preserves_a_reversible_separator() {
        let source = "left right";
        let current_byte = "left ".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("paragraph Enter should split at the word boundary");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "left \n\nright");
    }

    #[test]
    fn heading_enter_before_a_single_space_preserves_a_reversible_separator() {
        let source = "# left right";
        let current_byte = "# left".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("heading interior Enter should split at the word boundary");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "# left \nright");
    }

    #[test]
    fn paragraph_enter_inside_bold_closes_and_reopens_the_inline_element() {
        let source = "**left right**";
        let current_byte = "**left".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter inside bold text should preserve both styled fragments");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "**left** \n\n**right**");
        assert_eq!(augmentation.cursor_byte_after, "**left** \n\n**".len());
    }

    #[test]
    fn paragraph_enter_inside_nested_inline_elements_preserves_marker_order() {
        let source = "**left *nested text* right**";
        let current_byte = "**left *nested".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter inside nested inline styles should close and reopen every frame");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "**left *nested*** \n\n***text* right**");
    }

    #[test]
    fn paragraph_enter_inside_link_duplicates_the_link_wrapper() {
        let source = "[left right](https://example.com)";
        let current_byte = "[left".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter inside link text should preserve a valid link on both sides");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "[left](https://example.com) \n\n[right](https://example.com)");
    }

    #[test]
    fn paragraph_enter_inside_inline_code_duplicates_the_code_delimiters() {
        let source = "before `code text` after";
        let current_byte = "before `code".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter inside inline code should keep two valid code spans");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "before `code` \n\n`text` after");
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
    fn list_enter_at_indented_lazy_continuation_consumes_the_old_indent() {
        let source = "- item\n  continuation";
        let current_byte = "- item".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter should turn the continuation into a sibling item");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "- item\n- continuation");
    }

    #[test]
    fn blockquote_enter_at_indented_lazy_continuation_consumes_the_old_indent() {
        let source = "> first\n  second";
        let current_byte = "> first".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter should make the lazy quote continuation explicit");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "> first\n> second");
    }

    #[test]
    fn heading_enter_inside_blockquote_preserves_the_quote_container() {
        let source = "> # left right";
        let current_byte = "> # left".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("heading Enter should create a paragraph inside the quote");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "> # left \n> right");
    }

    #[test]
    fn heading_enter_inside_list_preserves_the_list_item_container() {
        let source = "- # left right";
        let current_byte = "- # left".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("heading Enter should create a paragraph inside the item");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "- # left \n  right");
    }

    #[test]
    fn blockquote_inside_list_takes_precedence_over_the_outer_item() {
        let source = "- > left right";
        let current_byte = "- > left".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("the inner quote should own Enter");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "- > left \n  > right");
    }

    #[test]
    fn table_row_insert_inside_blockquote_preserves_the_quote_prefix() {
        let source = "> | a |\n> |---|\n> | b |";
        let current_byte = source.rfind('b').expect("fixture contains body cell");

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter should append a quoted table row");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "> | a |\n> |---|\n> | b |\n> |  |");
    }

    #[test]
    fn table_inside_list_takes_precedence_and_preserves_item_indent() {
        let source = "- table\n\n  | a |\n  |---|\n  | b |";
        let current_byte = source.rfind('b').expect("fixture contains body cell");

        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("the nested table should own Enter");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "- table\n\n  | a |\n  |---|\n  | b |\n  |  |");
    }

    #[test]
    fn line_break_preserves_the_word_separator_and_is_backspace_reversible() {
        let source = "left right";
        let split_byte = "left".len();

        let line_break = augment_edit(source, split_byte, AugmentKind::LineBreak)
            .expect("Shift+Enter should create a Markdown hard break");
        let source_with_break = apply_augmentation_at(source, split_byte, &line_break);
        assert_eq!(source_with_break, "left \\\nright");

        let backspace =
            augment_edit(&source_with_break, line_break.cursor_byte_after, AugmentKind::Backspace)
                .expect("Backspace at the hard-break continuation should restore the source");
        assert_eq!(
            apply_augmentation_at(&source_with_break, line_break.cursor_byte_after, &backspace),
            source
        );
    }

    #[test]
    fn line_break_preserves_crlf_and_inline_element_boundaries() {
        let source = "**left right**\r\nnext";
        let current_byte = "**left".len();

        let augmentation = augment_edit(source, current_byte, AugmentKind::LineBreak)
            .expect("Shift+Enter should preserve styled fragments and CRLF");
        let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

        assert_eq!(edited_source, "**left** \\\r\n**right**\r\nnext");
    }

    #[test]
    fn inline_element_splits_are_backspace_reversible() {
        for (source, current_byte) in [
            ("**left right**", "**left".len()),
            ("**left *nested text* right**", "**left *nested".len()),
            ("[left right](https://example.com)", "[left".len()),
            ("before `code text` after", "before `code".len()),
        ] {
            let enter = augment_edit(source, current_byte, AugmentKind::Enter)
                .unwrap_or_else(|| panic!("Enter must split inline fixture {source:?}"));
            let split_source = apply_augmentation_at(source, current_byte, &enter);
            let backspace =
                augment_edit(&split_source, enter.cursor_byte_after, AugmentKind::Backspace)
                    .unwrap_or_else(|| panic!("Backspace must reverse inline fixture {source:?}"));

            assert_eq!(
                apply_augmentation_at(&split_source, enter.cursor_byte_after, &backspace),
                source,
                "inline Enter/Backspace must roundtrip {source:?}"
            );
            assert_eq!(backspace.cursor_byte_after, reversible_split_point(source, current_byte));
        }
    }

    #[test]
    fn styled_line_break_is_backspace_reversible() {
        let source = "**left right**";
        let current_byte = "**left".len();
        let line_break = augment_edit(source, current_byte, AugmentKind::LineBreak)
            .expect("Shift+Enter must split styled text");
        let split_source = apply_augmentation_at(source, current_byte, &line_break);
        let backspace =
            augment_edit(&split_source, line_break.cursor_byte_after, AugmentKind::Backspace)
                .expect("Backspace must reverse a styled hard break");

        assert_eq!(
            apply_augmentation_at(&split_source, line_break.cursor_byte_after, &backspace),
            source
        );
        assert_eq!(backspace.cursor_byte_after, reversible_split_point(source, current_byte));
    }

    #[test]
    fn inline_html_line_break_is_backspace_reversible() {
        let source = "# left right";
        let current_byte = "# left".len();
        let line_break = augment_edit(source, current_byte, AugmentKind::LineBreak)
            .expect("Shift+Enter must insert a heading line break");
        let split_source = apply_augmentation_at(source, current_byte, &line_break);
        let backspace =
            augment_edit(&split_source, line_break.cursor_byte_after, AugmentKind::Backspace)
                .expect("Backspace must remove the complete inline HTML break");

        assert_eq!(
            apply_augmentation_at(&split_source, line_break.cursor_byte_after, &backspace),
            source
        );
        assert_eq!(backspace.cursor_byte_after, current_byte);
    }

    #[test]
    fn inline_element_splits_are_delete_forward_reversible() {
        for (source, current_byte) in [
            ("**left right**", "**left".len()),
            ("**left *nested text* right**", "**left *nested".len()),
            ("[left right](https://example.com)", "[left".len()),
            ("before `code text` after", "before `code".len()),
        ] {
            let enter = augment_edit(source, current_byte, AugmentKind::Enter)
                .unwrap_or_else(|| panic!("Enter must split inline fixture {source:?}"));
            let split_source = apply_augmentation_at(source, current_byte, &enter);
            let delete_byte =
                inline_safe_split_point(source, reversible_split_point(source, current_byte));
            let delete = augment_delete_forward(&split_source, delete_byte)
                .unwrap_or_else(|| panic!("DeleteForward must reverse inline fixture {source:?}"));

            assert_eq!(
                apply_augmentation_at(&split_source, delete_byte, &delete),
                source,
                "inline Enter/DeleteForward must roundtrip {source:?}"
            );
            assert_eq!(delete.cursor_byte_after, delete_byte);
        }
    }

    #[test]
    fn inline_html_line_break_is_delete_forward_reversible() {
        let source = "# left right";
        let current_byte = "# left".len();
        let line_break = augment_edit(source, current_byte, AugmentKind::LineBreak)
            .expect("Shift+Enter must insert a heading line break");
        let split_source = apply_augmentation_at(source, current_byte, &line_break);
        let delete = augment_delete_forward(&split_source, current_byte)
            .expect("DeleteForward must remove the complete inline HTML break");

        assert_eq!(apply_augmentation_at(&split_source, current_byte, &delete), source);
        assert_eq!(delete.cursor_byte_after, current_byte);
    }

    #[test]
    fn line_break_preserves_list_and_quote_container_paths() {
        let list_source = "- left right";
        let list_cursor = "- left".len();
        let list_augmentation = augment_edit(list_source, list_cursor, AugmentKind::LineBreak)
            .expect("Shift+Enter should stay inside the list item");
        assert_eq!(
            apply_augmentation_at(list_source, list_cursor, &list_augmentation),
            "- left \\\n  right"
        );

        let quote_source = "- > left right";
        let quote_cursor = "- > left".len();
        let quote_augmentation = augment_edit(quote_source, quote_cursor, AugmentKind::LineBreak)
            .expect("Shift+Enter should stay inside the nested quote");
        assert_eq!(
            apply_augmentation_at(quote_source, quote_cursor, &quote_augmentation),
            "- > left \\\n  > right"
        );
    }

    #[test]
    fn list_line_break_uses_markdown_content_columns() {
        for (source, expected_source) in [
            ("-\tleft right", "-\tleft \\\n    right"),
            ("-   left right", "-   left \\\n    right"),
            ("10.\tleft right", "10.\tleft \\\n    right"),
            ("- parent\n  - left right", "- parent\n  - left \\\n    right"),
        ] {
            let current_byte = source.find("left").expect("fixture contains left") + "left".len();
            let augmentation = augment_edit(source, current_byte, AugmentKind::LineBreak)
                .expect("list line break must be augmented");

            assert_eq!(
                apply_augmentation_at(source, current_byte, &augmentation),
                expected_source,
                "wrong Markdown content column for {source:?}"
            );
        }
    }

    #[test]
    fn list_enter_preserves_multi_space_marker_separator() {
        let source = "-   left right";
        let current_byte = source.find("left").expect("fixture contains left") + "left".len();
        let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
            .expect("Enter must continue the list marker");

        assert_eq!(
            apply_augmentation_at(source, current_byte, &augmentation),
            "-   left \n-   right"
        );
    }

    #[test]
    fn paragraph_backspace_into_tabbed_list_aligns_to_content_column() {
        let source = "-\titem\n\nparagraph";
        let current_byte = source.find("paragraph").expect("fixture contains paragraph");
        let augmentation = augment_edit(source, current_byte, AugmentKind::Backspace)
            .expect("Backspace must merge the paragraph into the list");

        assert_eq!(
            apply_augmentation_at(source, current_byte, &augmentation),
            "-\titem\n    paragraph"
        );
    }

    #[test]
    fn line_break_uses_inline_html_where_a_physical_line_would_end_the_leaf() {
        for (source, current_byte) in [
            ("# left right", "# left".len()),
            ("| a |\n|---|\n| left right |", "| a |\n|---|\n| left".len()),
        ] {
            let augmentation = augment_edit(source, current_byte, AugmentKind::LineBreak)
                .expect("Shift+Enter should preserve the heading or table-cell leaf");
            let edited_source = apply_augmentation_at(source, current_byte, &augmentation);
            assert_eq!(&edited_source[current_byte..current_byte + 4], "<br>");
        }
    }

    #[test]
    fn metadata_enter_and_line_break_insert_indented_literal_newlines() {
        let source = "---\nsection:\n  title: hello\n---";
        let current_byte = source.find("hello").expect("fixture contains metadata value") + 2;

        for kind in [AugmentKind::Enter, AugmentKind::LineBreak] {
            let augmentation = augment_edit(source, current_byte, kind.clone())
                .expect("metadata editing must emit a literal source newline");
            let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

            assert_eq!(edited_source, "---\nsection:\n  title: he\n  llo\n---");
            assert!(!edited_source.contains("\\\n"));
        }
    }

    #[test]
    fn html_block_enter_and_line_break_insert_indented_literal_newlines() {
        let source = "<div>\n  hello world\n</div>";
        let current_byte = source.find("hello").expect("fixture contains HTML text") + 5;

        for kind in [AugmentKind::Enter, AugmentKind::LineBreak] {
            let augmentation = augment_edit(source, current_byte, kind.clone())
                .expect("HTML block editing must emit a literal source newline");
            let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

            assert_eq!(edited_source, "<div>\n  hello\n   world\n</div>");
            assert!(!edited_source.contains("\\\n"));
        }
    }

    #[test]
    fn line_break_on_empty_or_unknown_structural_line_never_inserts_hard_break_marker() {
        for (source, current_byte) in [("left\n\nright", "left\n".len()), ("---", 0)] {
            let augmentation = augment_edit(source, current_byte, AugmentKind::LineBreak)
                .expect("literal structural line break must produce an augmentation");
            let edited_source = apply_augmentation_at(source, current_byte, &augmentation);

            assert!(!edited_source.contains("\\\n"), "unexpected hard break in {edited_source:?}");
        }
    }

    #[test]
    fn enter_on_an_empty_nested_container_exits_exactly_one_level() {
        for (source, expected_source) in [("- > ", "- "), ("> - ", "> "), ("> > ", "> ")] {
            let augmentation = augment_edit(source, source.len(), AugmentKind::Enter)
                .expect("Enter should exit the innermost empty container");
            let edited_source = apply_augmentation_at(source, source.len(), &augmentation);

            assert_eq!(edited_source, expected_source, "wrong exit level for {source:?}");
            assert_eq!(augmentation.cursor_byte_after, expected_source.len());
        }
    }

    #[test]
    fn setext_heading_enter_preserves_outer_container_path() {
        for (source, expected_source) in [
            ("> Title\n> =====", "> Title\n> =====\n> "),
            ("- Title\n  =====", "- Title\n  =====\n  "),
        ] {
            let current_byte =
                source.find("Title").expect("fixture contains title") + "Title".len();
            let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
                .expect("Enter should append a paragraph inside the same container");

            assert_eq!(apply_augmentation_at(source, current_byte, &augmentation), expected_source);
        }
    }

    #[test]
    fn table_empty_row_exit_preserves_outer_container_path() {
        for (source, expected_source) in [
            ("> | a |\n> |---|\n> |  |", "> | a |\n> |---|\n> "),
            ("- table\n\n  | a |\n  |---|\n  |  |", "- table\n\n  | a |\n  |---|\n  "),
        ] {
            let current_byte = source.rfind('|').expect("fixture contains empty row") - 1;
            let augmentation = augment_edit(source, current_byte, AugmentKind::Enter)
                .expect("Enter should leave the table but keep its parent container");

            assert_eq!(apply_augmentation_at(source, current_byte, &augmentation), expected_source);
        }
    }

    #[test]
    fn delete_forward_before_a_sibling_block_line_is_consumed() {
        // 段末 Delete 若会把下一行并上来自成 setext 标题/破坏 ATX/围栏/列表/引用,
        // 必须拦截为消费型空操作(不删任何字节)。
        for (source, cursor) in [
            ("文字\n\n---", "文字".len()),
            ("文字\r\n\r\n---", "文字".len()),
            ("文字\n\n===", "文字".len()),
            ("文字\n\n# 标题", "文字".len()),
            ("文字\n\n```", "文字".len()),
            ("文字\n\n- item", "文字".len()),
            ("文字\n\n> 引用", "文字".len()),
            ("- a\n\n- b", "- a".len()),
            ("| a |\n|---|---|\n| b |", "| a |".len()),
        ] {
            let augmentation = augment_delete_forward(source, cursor)
                .unwrap_or_else(|| panic!("DeleteForward at {source:?} must be guarded"));
            let edited_source = apply_augmentation_at(source, cursor, &augmentation);

            assert_eq!(edited_source, source, "DeleteForward must not alter {source:?}");
            assert_eq!(augmentation.cursor_byte_after, cursor);
        }
    }

    #[test]
    fn line_column_position_preserves_only_container_tab_overshoot() {
        for (required_columns, expected_residual) in [(4, 0), (3, 1), (2, 2), (1, 3)] {
            let source = "\t=";
            let path = ContainerPath(vec![ContainerSegment::IndentColumns(required_columns)]);
            let container_position = matching_container_position(source, 0, source.len(), &path)
                .expect("tab must reach the container target");
            assert_eq!(container_position.leaf_indent_columns(), Some(expected_residual));
            assert!(
                marker_position_after_optional_indent(source, source.len(), container_position)
                    .is_some(),
                "container overshoot {expected_residual} must be a valid leaf indent"
            );
        }

        let source = "\t=";
        let root_position = LineColumnPosition::at_line_start(0);
        assert!(
            marker_position_after_optional_indent(source, source.len(), root_position).is_none(),
            "a root-level tab is four leaf-indent columns"
        );

        let source = "  \t=";
        let path = ContainerPath(vec![ContainerSegment::IndentColumns(2)]);
        let exact_container_position = matching_container_position(source, 0, source.len(), &path)
            .expect("spaces exactly reach the container target");
        assert!(
            marker_position_after_optional_indent(source, source.len(), exact_container_position)
                .is_none(),
            "a tab beginning after the container target is four leaf-indent columns"
        );
    }

    #[test]
    fn block_quote_segment_consumes_container_tab_residual_as_optional_indent() {
        for (required_columns, explicit_spaces) in [(3, 2), (2, 1), (1, 0)] {
            let source = format!("\t{}> marker", " ".repeat(explicit_spaces));
            let path = ContainerPath(vec![
                ContainerSegment::IndentColumns(required_columns),
                ContainerSegment::BlockQuote,
            ]);
            let position = matching_container_position(&source, 0, source.len(), &path)
                .unwrap_or_else(|| panic!("container path must match {source:?}"));

            assert_eq!(source.as_bytes().get(position.byte_offset), Some(&b'm'));
            assert_eq!(position.absolute_column, position.container_target_column);
        }

        let source = "\t  > marker";
        let path =
            ContainerPath(vec![ContainerSegment::IndentColumns(2), ContainerSegment::BlockQuote]);
        assert!(
            matching_container_position(source, 0, source.len(), &path).is_none(),
            "two residual columns plus two physical spaces exceed quote optional indent"
        );
    }

    #[test]
    fn delete_forward_at_paragraph_end_merges_the_following_paragraph() {
        for (source, cursor, expected_source) in [
            ("a\n\nb", 1, "ab"),
            ("a\r\n\r\nb", 1, "ab"),
            ("first\nsecond", "first".len(), "firstsecond"),
            ("# 标题\n\n正文", "# 标题".len(), "# 标题正文"),
        ] {
            let augmentation = augment_delete_forward(source, cursor)
                .unwrap_or_else(|| panic!("DeleteForward should merge paragraphs in {source:?}"));
            let edited_source = apply_augmentation_at(source, cursor, &augmentation);

            assert_eq!(edited_source, expected_source, "wrong merge result for {source:?}");
            assert_eq!(augmentation.cursor_byte_after, cursor);
        }
    }

    #[test]
    fn delete_forward_inside_code_block_body_falls_back_to_default() {
        // 代码体内的下一行只是代码文本(即使以 `#` 开头),不做块边界拦截。
        let source = "```\nlet a\n# b\n```";
        let cursor = "```\nlet a".len();

        assert!(augment_delete_forward(source, cursor).is_none());
    }

    #[test]
    fn delete_forward_before_code_block_closing_fence_is_consumed() {
        let source = "```\ncode\n```\n\npara";
        let cursor = "```\ncode".len();

        let augmentation = augment_delete_forward(source, cursor)
            .expect("DeleteForward must not merge the closing fence into code");
        let edited_source = apply_augmentation_at(source, cursor, &augmentation);

        assert_eq!(edited_source, source);
        assert_eq!(augmentation.cursor_byte_after, cursor);
    }

    #[test]
    fn delete_forward_protects_short_setext_underlines() {
        for source in ["title\n=", "title\n==", "title\n--"] {
            let cursor = "title".len();
            let augmentation = augment_delete_forward(source, cursor)
                .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

            assert_eq!(augmentation.replace_range, None, "must not merge {source:?}");
            assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
        }
    }

    #[test]
    fn delete_forward_allows_marker_like_paragraphs() {
        for (source, expected_source) in [
            ("first\n---not-a-rule", "first---not-a-rule"),
            ("first\n===not-setext", "first===not-setext"),
            ("first\n***plain", "first***plain"),
        ] {
            let cursor = "first".len();
            let augmentation = augment_delete_forward(source, cursor)
                .unwrap_or_else(|| panic!("DeleteForward must merge {source:?}"));

            assert_eq!(augmentation.replace_range, Some(cursor..cursor + 1));
            assert_eq!(apply_augmentation_at(source, cursor, &augmentation), expected_source);
        }
    }

    #[test]
    fn delete_forward_inside_fenced_code_only_protects_matching_closer() {
        for source in
            ["````\ncode\n```\n````", "~~~~\ncode\n```\n~~~~", "```\ncode\n```not-close\n```"]
        {
            let cursor = source.find("\ncode").expect("fixture contains code") + "\ncode".len();

            assert!(
                augment_delete_forward(source, cursor).is_none(),
                "non-closing fence line must stay default for {source:?}"
            );
        }

        let source = "```\ncode\n``` \t\npara";
        let cursor = "```\ncode".len();
        let augmentation = augment_delete_forward(source, cursor)
            .expect("matching closing fence with trailing whitespace must be protected");

        assert_eq!(augmentation.replace_range, None);
        assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
    }

    #[test]
    fn delete_forward_inside_container_fenced_code_matches_the_real_closer() {
        for source in [
            "> ```\n> code\n>````\n> paragraph",
            "> ```\n> code\n> ````\n> paragraph",
            " > ```\n> code\n  >````\n> paragraph",
            "- ```\n   code\n   ````\n\nparagraph",
            "- ```\n  code\n     ````\n\nparagraph",
            "- ```\n  code\n  ````\n\nparagraph",
            "-\t```\n\tcode\n\t````\n\nparagraph",
            "- item\n\n  ```\n  code\n     ````\n\nparagraph",
            "- item\n\n  ```\n  code\n\t```\n\nparagraph",
            "> 12. ```\n>     code\n> \t\t``` \n\nparagraph",
        ] {
            let cursor = source.find("code").expect("fixture contains code") + "code".len();
            let augmentation = augment_delete_forward(source, cursor)
                .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

            assert_eq!(augmentation.replace_range, None);
            assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
        }
    }

    #[test]
    fn delete_forward_matches_list_quote_fences_across_tab_residual() {
        for source in
            ["- > ```\n  > code\n\t> ```\n", "- item\n\n\t> ```\n\t> code\n  > ```\n\nparagraph"]
        {
            let cursor = source.find("code").expect("fixture contains code") + "code".len();
            let augmentation = augment_delete_forward(source, cursor)
                .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

            assert_eq!(augmentation.replace_range, None);
            assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
        }
    }

    #[test]
    fn delete_forward_inside_container_fenced_code_allows_pseudo_closers() {
        for source in [
            "> ```\n> code\n> ```not-close\n> ```",
            "> ```\n> code\n```\nparagraph",
            "- ```\n  code\n  ```not-close\n  ```",
            "- ```\n  code\n```\nparagraph",
        ] {
            let cursor = source.find("code").expect("fixture contains code") + "code".len();

            assert!(
                augment_delete_forward(source, cursor).is_none(),
                "pseudo closer must stay default for {source:?}"
            );
        }
    }

    #[test]
    fn delete_forward_inside_fenced_code_rejects_over_indented_pseudo_closers() {
        for source in [
            "```\ncode\n    ```\n```",
            "```\ncode\n\t```\n```",
            "> ```\n> code\n>     ```\n> ```",
            "> ```\n> code\n> \t```\n> ```",
            "- ```\n  code\n      ```\n  ```",
            "- ```\n  code\n  \t```\n  ```",
            "- item\n\n  ```\n  code\n      ```\n  ```",
        ] {
            let cursor = source.find("code").expect("fixture contains code") + "code".len();

            assert!(
                augment_delete_forward(source, cursor).is_none(),
                "over-indented pseudo closer must stay default for {source:?}"
            );
        }
    }

    #[test]
    fn delete_forward_inside_code_before_blank_line_keeps_default() {
        let source = "```\ncode\n\n```";
        let cursor = "```\ncode".len();

        assert!(
            augment_delete_forward(source, cursor).is_none(),
            "DeleteForward must remove only the first newline before a blank code line"
        );
    }

    #[test]
    fn fenced_code_start_range_points_to_the_opening_marker() {
        use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag};

        for source in ["```\ncode\n```", "> ```\n> code\n> ```", "- ```\n  code\n  ```"] {
            let marker_start = Parser::new_ext(source, crate::parser::markdown_options())
                .into_offset_iter()
                .find_map(|(event, range)| {
                    matches!(event, Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))))
                        .then_some(range.start)
                })
                .expect("fixture contains fenced code");

            assert_eq!(marker_start, source.find("```").expect("fixture contains opener"));
        }
    }

    #[test]
    fn delete_forward_rejects_indented_code_as_thematic_break() {
        for source in ["first\n\t***", "first\n    ***"] {
            let cursor = "first".len();
            let augmentation = augment_delete_forward(source, cursor)
                .unwrap_or_else(|| panic!("DeleteForward must merge {source:?}"));

            assert_eq!(augmentation.replace_range, Some(cursor..cursor + 1));
            assert_eq!(
                apply_augmentation_at(source, cursor, &augmentation),
                source.replacen('\n', "", 1)
            );
        }
    }

    #[test]
    fn delete_forward_accepts_valid_thematic_break_indentation() {
        for source in ["first\n***", "first\n *\t*\t*", "first\n  ***", "first\n   ***"] {
            let cursor = "first".len();
            let augmentation = augment_delete_forward(source, cursor)
                .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

            assert_eq!(augmentation.replace_range, None);
            assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
        }
    }

    #[test]
    fn delete_forward_marker_boundaries_support_crlf() {
        for source in ["title\r\n=", "title\r\n  ==\t", "first\r\n *\t*\t*"] {
            let cursor = source.find("\r\n").expect("fixture contains CRLF");
            let augmentation = augment_delete_forward(source, cursor)
                .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

            assert_eq!(augmentation.replace_range, None);
        }

        let source = "first\r\n===not-setext";
        let cursor = "first".len();
        let augmentation =
            augment_delete_forward(source, cursor).expect("marker-like CRLF paragraph must merge");
        assert_eq!(augmentation.replace_range, Some(cursor..cursor + 2));
        assert_eq!(apply_augmentation_at(source, cursor, &augmentation), "first===not-setext");

        let source = "````\r\ncode\r\n````` \t\r\nparagraph";
        let cursor = source.find("code").expect("fixture contains code") + "code".len();
        let augmentation = augment_delete_forward(source, cursor)
            .expect("longer CRLF closing fence must be protected");
        assert_eq!(augmentation.replace_range, None);
    }

    #[test]
    fn delete_forward_protects_markers_after_wide_list_continuation() {
        for source in ["123. title\n     =", "123. para\n     ***"] {
            let cursor = source.find('\n').expect("fixture contains newline");
            let augmentation = augment_delete_forward(source, cursor)
                .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

            assert_eq!(augmentation.replace_range, None);
            assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
        }
    }

    #[test]
    fn delete_forward_protects_markers_after_nested_container_continuation() {
        for source in [
            "123. > title\n     > =",
            "123. > para\n     > ***",
            "> 123. title\n>      =",
            "> 123. para\n>      ***",
            "- title\n\t=",
            "- para\n\t***",
            "> - title\n>\t=",
        ] {
            let cursor = source.find('\n').expect("fixture contains newline");
            let augmentation = augment_delete_forward(source, cursor)
                .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

            assert_eq!(augmentation.replace_range, None);
            assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
        }
    }

    #[test]
    fn delete_forward_protects_list_quote_markers_across_tab_residual() {
        for source in ["- > title\n\t> =", "- > para\n\t> ***"] {
            let cursor = source.find('\n').expect("fixture contains newline");
            let augmentation = augment_delete_forward(source, cursor)
                .unwrap_or_else(|| panic!("DeleteForward must protect {source:?}"));

            assert_eq!(augmentation.replace_range, None);
            assert_eq!(apply_augmentation_at(source, cursor, &augmentation), source);
        }
    }

    #[test]
    fn delete_forward_rejects_over_indented_or_marker_like_container_lines() {
        for source in [
            "123. para\n         ***",
            "123. para\n     \t***",
            "123. para\n     ***plain",
            "123. title\n     ===not-setext",
            "123. > para\n     >     ***",
            "123. > para\n     > \t***",
        ] {
            let cursor = source.find('\n').expect("fixture contains newline");
            let augmentation = augment_delete_forward(source, cursor);

            assert!(
                augmentation.as_ref().is_none_or(|value| value.replace_range.is_some()),
                "DeleteForward must not guard {source:?}"
            );
        }
    }

    #[test]
    fn delete_forward_after_hard_break_marker_keeps_default_behaviour() {
        // 行尾硬换行标记属于段内结构,Delete 仍逐字符削弱标记。
        let source = "first  \n\nsecond";
        let cursor = "first".len();

        assert!(augment_delete_forward(source, cursor).is_none());
    }

    #[test]
    fn enter_on_opening_fence_line_moves_cursor_into_code_body() {
        // 开头围栏行(含 info string)任意位置回车:在该行行尾插入单个换行,
        // 光标进入代码体第一行,围栏本身保持完整。
        for (source, cursor) in [
            ("```rust\ncode\n```", 0),
            ("```rust\ncode\n```", 5),
            ("```rust\ncode\n```", "```rust".len()),
            ("~~~\ncode\n~~~", "~~~".len()),
            // 未闭合围栏(代码块延伸到 EOF)只有开头围栏行情形。
            ("```rust\ncode", 5),
        ] {
            let fence_line_end = source.find('\n').unwrap_or(source.len());
            let augmentation = augment_edit(source, cursor, AugmentKind::Enter)
                .unwrap_or_else(|| panic!("Enter on opening fence line of {source:?}"));
            let edited_source = apply_augmentation_at(source, cursor, &augmentation);
            let mut expected_source = source.to_owned();
            expected_source.insert(fence_line_end, '\n');

            assert_eq!(edited_source, expected_source, "wrong opening fence enter for {source:?}");
            assert_eq!(augmentation.cursor_byte_after, fence_line_end + 1);
        }
    }

    #[test]
    fn enter_on_closing_fence_line_exits_code_block() {
        // 闭合围栏行任意位置回车:在闭合围栏行尾建立块边界,光标落到新空段。
        for (source, cursor, expected_source) in [
            ("```\ncode\n```", 9, "```\ncode\n```\n\n"),
            ("```\ncode\n```", "```\ncode\n```".len(), "```\ncode\n```\n\n"),
            ("~~~\ncode\n~~~", 11, "~~~\ncode\n~~~\n\n"),
            ("```\ncode\n```\npara", 9, "```\ncode\n```\n\n\npara"),
        ] {
            let augmentation = augment_edit(source, cursor, AugmentKind::Enter)
                .unwrap_or_else(|| panic!("Enter on closing fence line of {source:?}"));
            let edited_source = apply_augmentation_at(source, cursor, &augmentation);

            assert_eq!(edited_source, expected_source, "wrong closing fence enter for {source:?}");
            assert_eq!(augmentation.cursor_byte_after, "```\ncode\n```\n\n".len());
        }
    }

    #[test]
    fn enter_inside_code_block_body_keeps_default_newline() {
        // 代码体内部(含形似短围栏的代码行)回车仍回落默认裸换行。
        for (source, cursor) in [
            ("```\ncode\n```", 6),
            ("````\n```\n````", 7),
            ("````\ncode\n```", "````\ncode\n```".len()),
        ] {
            assert!(
                augment_edit(source, cursor, AugmentKind::Enter).is_none(),
                "code body Enter must stay default for {source:?}"
            );
        }
    }

    #[test]
    fn delete_forward_after_closing_fence_line_is_consumed_at_tight_boundary() {
        // 闭合围栏行尾紧贴下一段(单个换行)时 Delete 必须拦截;
        // 有空行兜底(≥2 个换行)时交回默认删除。
        let tight_source = "```\ncode\n```\npara";
        let tight_cursor = "```\ncode\n```".len();
        let augmentation = augment_delete_forward(tight_source, tight_cursor)
            .expect("DeleteForward must not merge a paragraph into the closing fence");

        assert_eq!(apply_augmentation_at(tight_source, tight_cursor, &augmentation), tight_source);
        assert_eq!(augmentation.cursor_byte_after, tight_cursor);

        let spaced_source = "```\ncode\n```\n\npara";
        assert!(augment_delete_forward(spaced_source, tight_cursor).is_none());
    }

    #[test]
    fn list_item_hosting_only_a_nested_list_is_not_empty() {
        // `- ` 下只挂子列表时父 item 不算空:父行回车不得删掉 `- ` 把子列表提升为顶层。
        let source = "- \n  - x";
        let cursor = "- ".len();

        let context = classify_enter_context(source, cursor);
        let EnterContext::ListItem { empty, .. } = context else {
            panic!("expected ListItem context, got {context:?}");
        };
        assert!(!empty, "parent item hosting a nested list must not be empty");

        let augmentation = augment_edit(source, cursor, AugmentKind::Enter)
            .expect("Enter on the parent line should produce an augmentation");
        let edited_source = apply_augmentation_at(source, cursor, &augmentation);
        assert!(
            edited_source.starts_with("- \n"),
            "parent marker must survive Enter, got {edited_source:?}"
        );
    }

    #[test]
    fn enter_on_empty_nested_list_item_exits_one_level_at_a_time() {
        // 嵌套空 item 回车退一层(行替换为父级前缀);残留行再回车继续退出,
        // 最终落在干净的空行,而不是意外创建同级 item。
        // 注意:空嵌套 item 必须跟在非空嵌套 item 之后,否则 `- ` 会被
        // pulldown-cmark 解析成上一行文本的 setext 下划线。
        let source = "- a\n  - b\n    - c\n    - ";
        let mut current_source = source.to_owned();
        let mut cursor = current_source.len();
        for expected_source in
            ["- a\n  - b\n    - c\n    ", "- a\n  - b\n    - c\n  ", "- a\n  - b\n    - c\n"]
        {
            let augmentation = augment_edit(&current_source, cursor, AugmentKind::Enter)
                .expect("Enter should exit one list level");
            current_source = apply_augmentation_at(&current_source, cursor, &augmentation);
            cursor = augmentation.cursor_byte_after;

            assert_eq!(current_source, expected_source);
            assert_eq!(cursor, expected_source.len());
        }
    }

    #[test]
    fn enter_inside_list_marker_behaves_like_enter_at_content_start() {
        // 光标在 marker 内部(`-| item`)回车 = 在内容起点回车,
        // 不得产出 `-\n item` 这样的懒延续残留。
        for (source, cursor, expected_source, expected_cursor) in [
            ("- item", "-".len(), "- \n- item", "- \n- ".len()),
            ("1. item", "1.".len(), "1. \n2. item", "1. \n2. ".len()),
        ] {
            let augmentation =
                augment_edit(source, cursor, AugmentKind::Enter).unwrap_or_else(|| {
                    panic!("Enter inside the marker of {source:?} should continue the list")
                });
            let edited_source = apply_augmentation_at(source, cursor, &augmentation);

            assert_eq!(
                edited_source, expected_source,
                "wrong marker-interior enter for {source:?}"
            );
            assert_eq!(augmentation.cursor_byte_after, expected_cursor);
        }
    }

    #[test]
    fn heading_content_start_scans_actual_whitespace_after_hashes() {
        // `#` 后的多余空白不属于标题内容:光标落在空白内时 Enter 回落默认。
        for (source, cursor) in [("#  Title", 2), ("#\t\tTitle", 2), ("##   Title", 3)] {
            assert!(
                augment_edit(source, cursor, AugmentKind::Enter).is_none(),
                "Enter inside heading marker whitespace must stay default for {source:?}"
            );
        }

        // 内容起点回车前插普通空段，原标题保持完整。
        let source = "#  Title";
        let cursor = "#  ".len();
        let augmentation = augment_edit(source, cursor, AugmentKind::Enter)
            .expect("Enter at heading content start should insert a preceding paragraph");
        assert_eq!(apply_augmentation_at(source, cursor, &augmentation), "\n#  Title");
        assert_eq!(augmentation.cursor_byte_after, cursor + LF_SEQUENCE.len());

        // 空标题(`#` 后无空白)行尾回车 = 退出到新段落。
        let empty_heading = "#";
        let augmentation = augment_edit(empty_heading, 1, AugmentKind::Enter)
            .expect("Enter on an empty heading should create a paragraph break");
        assert_eq!(apply_augmentation_at(empty_heading, 1, &augmentation), "#\n\n");
        assert_eq!(augmentation.cursor_byte_after, 3);
    }
}
