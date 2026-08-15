//! Markdown 感知的编辑增强（Enter / Backspace / InsertText）。
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

use crate::builder::ListBullet;
use ui::plugin::{AugmentKind, EditAugmentation};
use unicode_segmentation::UnicodeSegmentation;

const LF_SEQUENCE: &str = "\n";
const CRLF_SEQUENCE: &str = "\r\n";
const BLOCK_BOUNDARY_NEWLINE_COUNT: usize = 2;
const MAX_LEADING_BLOCK_INDENT: usize = 3;

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
        AugmentKind::Backspace => augment_backspace(source, current_byte),
        AugmentKind::Tab => None,
        AugmentKind::InsertText(ref text) => augment_insert_text(source, current_byte, text),
    }
}

fn augment_enter(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    let context = classify_enter_context(source, current_byte);
    enter_context_augmentation(source, current_byte, context)
}

fn augment_backspace(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    if let Some(aug) = backspace_empty_source_line(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_paragraph_boundary(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_last_interblock_paragraph_grapheme(source, current_byte) {
        return Some(aug);
    }
    let range = get_marker_delete_range(source, current_byte)?;
    Some(EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(range.clone()),
        cursor_byte_after: range.start,
    })
}

fn backspace_last_interblock_paragraph_grapheme(
    source: &str,
    current_byte: usize,
) -> Option<EditAugmentation> {
    if !matches!(classify_enter_context(source, current_byte), EnterContext::TopLevelParagraphEnd) {
        return None;
    }

    let (line_start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    let content_end = source_line_content_end(source, line_end);
    if current_byte != content_end
        || UnicodeSegmentation::graphemes(&source[line_start..content_end], true).count() != 1
        || !has_two_newline_sequences_before(source, line_start)
        || !has_two_newline_sequences_after(source, content_end)
    {
        return None;
    }

    let trailing_newline_width = newline_sequence_width_at(source, content_end)?;
    let aug = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(line_start..content_end + trailing_newline_width),
        cursor_byte_after: line_start,
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}

fn has_two_newline_sequences_before(source: &str, byte: usize) -> bool {
    let Some(first_width) = newline_sequence_width_before(source, byte) else {
        return false;
    };
    newline_sequence_width_before(source, byte - first_width).is_some()
}

fn has_two_newline_sequences_after(source: &str, byte: usize) -> bool {
    let Some(first_width) = newline_sequence_width_at(source, byte) else {
        return false;
    };
    newline_sequence_width_at(source, byte + first_width).is_some()
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
    let context = classify_enter_context(source, current_byte);
    if !matches!(context, EnterContext::EmptyBlockSeparatorLine) {
        return None;
    }
    // 在"块间空行 run"里输入字符：把插入点周围的换行修剪为两侧各恰好 2 个
    // （保持前后块间距不变），中间放入 text；如触及文档边界则不再强补换行。
    let mut start = current_byte;
    let mut left_newline_count = 0;
    while let Some(width) = newline_sequence_width_before(source, start) {
        start -= width;
        left_newline_count += 1;
    }
    let mut end = current_byte;
    let mut right_newline_count = 0;
    while let Some(width) = newline_sequence_width_at(source, end) {
        end += width;
        right_newline_count += 1;
    }

    let at_start = start == 0;
    let at_end = end == source.len();

    let needed_left = if at_start {
        left_newline_count
    } else {
        left_newline_count.max(BLOCK_BOUNDARY_NEWLINE_COUNT)
    };
    let needed_right = if at_end {
        right_newline_count
    } else {
        right_newline_count.max(BLOCK_BOUNDARY_NEWLINE_COUNT)
    };
    if needed_left == 0 && needed_right == 0 {
        return None;
    }

    let newline = preferred_newline_sequence(source, current_byte);
    let mut insert =
        String::with_capacity((needed_left + needed_right) * newline.len() + text.len());
    for _ in 0..needed_left {
        insert.push_str(newline);
    }
    insert.push_str(text);
    let cursor_after = start + insert.len();
    for _ in 0..needed_right {
        insert.push_str(newline);
    }

    let aug = EditAugmentation {
        replace_range: Some(start..end),
        insert_text: Some(insert),
        cursor_byte_after: cursor_after,
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
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

/// 在当前位置插入 `\n\n`，跨越 CommonMark 块边界（"切两半"）。
///
/// - 通用情况：插入 `"\n\n"`，光标跳 +2。
/// - 当 `bytes[current_byte] == '\n'`：如果这个 `\n` 已经是块分隔的一部分
///   或文档末尾，插入 `"\n"` 补一个空源码行；否则仍插入 `"\n\n"`。
///   两种情况下光标都固定跳到 `current_byte + 2`（下一段应有的位置）。
fn emit_block_break(source: &str, current_byte: usize) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, current_byte);
    let next_newline_width = newline_sequence_width_at(source, current_byte);
    let touches_next_newline = next_newline_width.is_some();
    let next_newline_end = current_byte + next_newline_width.unwrap_or(0);
    let next_is_blank_separator =
        touches_next_newline && newline_sequence_width_at(source, next_newline_end).is_some();
    let insertion_count =
        if touches_next_newline && (next_newline_end == source.len() || next_is_blank_separator) {
            1
        } else {
            BLOCK_BOUNDARY_NEWLINE_COUNT
        };
    let insertion = newline.repeat(insertion_count);
    let aug = EditAugmentation {
        cursor_byte_after: current_byte + newline.len() * BLOCK_BOUNDARY_NEWLINE_COUNT,
        insert_text: Some(insertion),
        ..Default::default()
    };
    debug_assert_augmentation(&aug, source);
    aug
}

fn backspace_empty_source_line(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    if line_start != line_end {
        return None;
    }

    let previous_line_end = previous_non_empty_line_end(source, current_byte)?;
    let delete_range = if current_byte >= source.len() {
        previous_line_end..current_byte
    } else if source.as_bytes().get(current_byte) == Some(&b'\n') {
        current_byte..current_byte + 1
    } else {
        current_byte.checked_sub(1)?..current_byte
    };
    let aug = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(delete_range),
        cursor_byte_after: previous_line_end,
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}

fn backspace_paragraph_boundary(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    if current_byte != line_start
        || line_start == line_end
        || !matches!(
            classify_enter_context(source, current_byte),
            EnterContext::TopLevelParagraphEnd | EnterContext::ParagraphInterior
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
    if boundary_start == current_byte {
        return None;
    }
    if !matches!(
        classify_enter_context(source, boundary_start),
        EnterContext::TopLevelParagraphEnd
            | EnterContext::ParagraphInterior
            | EnterContext::Heading { .. }
    ) {
        return None;
    }

    let aug = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(boundary_start..current_byte),
        cursor_byte_after: boundary_start,
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}

fn previous_non_empty_line_end(source: &str, current_byte: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut line_end = current_byte.min(source.len());
    while line_end > 0 {
        let line_start = bytes[..line_end]
            .iter()
            .rposition(|&source_byte| source_byte == b'\n')
            .map_or(0, |newline_index| newline_index + 1);
        if line_start < line_end {
            return Some(line_end);
        }
        line_end = line_start.checked_sub(1)?;
    }
    Some(0)
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

/// 删除当前源码行（含 range）。用于 list/blockquote 空 item 的"退出"。
fn emit_remove_current_line(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    let (start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    let content_end = source_line_content_end(source, line_end);
    let aug = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(start..content_end),
        cursor_byte_after: start,
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
        EnterContext::Heading { level: _, at_end } => {
            heading_enter_augmentation(source, current_byte, at_end)
        }
        EnterContext::ListItem { indent, bullet, empty, at_end: _ } => {
            list_item_enter_augmentation(source, current_byte, &indent, bullet, empty)
        }
        EnterContext::BlockQuoteLine { continuation_prefix, empty } => {
            blockquote_enter_augmentation(source, current_byte, &continuation_prefix, empty)
        }
        EnterContext::TableCell { next_cell_start } => Some(EditAugmentation {
            insert_text: Some(String::new()),
            replace_range: None,
            cursor_byte_after: next_cell_start.unwrap_or(current_byte),
        }),
        EnterContext::EmptyBlockSeparatorLine => Some(emit_source_newline(source, current_byte)),
        EnterContext::CodeBlock | EnterContext::Other => None,
    }
}

fn paragraph_enter_augmentation(source: &str, current_byte: usize) -> EditAugmentation {
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

fn heading_enter_augmentation(
    source: &str,
    current_byte: usize,
    at_end: bool,
) -> Option<EditAugmentation> {
    if at_end {
        return Some(emit_block_break(source, current_byte));
    }

    // Heading 中间：在当前光标处分割标题，后半段成为普通段落。
    let insertion = String::from(preferred_newline_sequence(source, current_byte));
    let aug = EditAugmentation {
        insert_text: Some(insertion.clone()),
        replace_range: Some(current_byte..current_byte),
        cursor_byte_after: current_byte + insertion.len(),
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}

fn list_item_enter_augmentation(
    source: &str,
    current_byte: usize,
    indent: &str,
    bullet: ListBullet,
    empty: bool,
) -> Option<EditAugmentation> {
    if empty {
        return emit_remove_current_line(source, current_byte);
    }
    let marker = match bullet {
        ListBullet::Bullet => String::from("- "),
        ListBullet::Ordered(n) => format!("{}. ", n + 1),
        ListBullet::TaskList(false) => String::from("- [ ] "),
        ListBullet::TaskList(true) => String::from("- [x] "),
    };
    Some(emit_marker_break(source, current_byte, indent, &marker))
}

fn blockquote_enter_augmentation(
    source: &str,
    current_byte: usize,
    continuation_prefix: &str,
    empty: bool,
) -> Option<EditAugmentation> {
    if empty {
        return emit_remove_current_line(source, current_byte);
    }
    Some(emit_marker_break(source, current_byte, "", continuation_prefix))
}

fn cursor_touches_source_newline(source: &str, current_byte: usize) -> bool {
    newline_sequence_width_at(source, current_byte).is_some()
        || newline_sequence_width_before(source, current_byte).is_some()
}

// ─── 分类器 ────────────────────────────────────────────────────────────────

/// Enter 键的上下文分类。分类结果决定 `enter_context_augmentation` 的分支。
#[derive(Debug)]
pub enum EnterContext {
    TopLevelParagraphEnd,
    ParagraphInterior,
    Heading { level: u8, at_end: bool },
    ListItem { indent: String, bullet: ListBullet, empty: bool, at_end: bool },
    BlockQuoteLine { continuation_prefix: String, empty: bool },
    CodeBlock,
    TableCell { next_cell_start: Option<usize> },
    EmptyBlockSeparatorLine,
    Other,
}

struct ItemFrame {
    start: usize,
    marker_end: usize,
    indent: String,
    bullet: ListBullet,
    saw_content: bool,
}

struct TableFrame {
    cell_ranges: Vec<Vec<std::ops::Range<usize>>>,
}

fn classify_heading_hit(
    source: &str,
    current_byte: usize,
    level: u8,
    start: usize,
    range: &std::ops::Range<usize>,
) -> EnterContext {
    let end = content_end_without_trailing_newline(source, start..range.end);
    let hash_prefix = level as usize + 1; // "# " / "## " / ...
    let content_start = start.saturating_add(hash_prefix);
    let at_end = current_byte == end;
    if current_byte >= content_start && current_byte <= end {
        EnterContext::Heading { level, at_end }
    } else {
        EnterContext::Other
    }
}

pub fn classify_enter_context(source: &str, current_byte: usize) -> EnterContext {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    let mark_item_content_seen = |stack: &mut Vec<ItemFrame>| {
        if let Some(frame) = stack.last_mut() {
            frame.saw_content = true;
        }
    };

    let parser = Parser::new_ext(source, pulldown_cmark::Options::all());
    let mut item_stack: Vec<ItemFrame> = Vec::new();
    let mut table: Option<TableFrame> = None;
    let mut heading_level: Option<u8> = None;
    let mut heading_start: Option<usize> = None;
    let mut paragraph_start: Option<(usize, usize, usize)> = None;
    let mut code_block_range: Option<std::ops::Range<usize>> = None;
    let mut blockquote_depth: usize = 0;

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::Item) => {
                let indent = list_item_indent(source, range.start);
                if let Some((bullet, marker_end)) = parse_list_marker(source, range.start) {
                    item_stack.push(ItemFrame {
                        start: range.start,
                        marker_end,
                        indent,
                        bullet,
                        saw_content: false,
                    });
                } else {
                    item_stack.push(ItemFrame {
                        start: range.start,
                        marker_end: range.start,
                        indent,
                        bullet: ListBullet::Bullet,
                        saw_content: false,
                    });
                }
            }
            Event::End(TagEnd::Item) => {
                if let Some(frame) = item_stack.pop()
                    && frame.marker_end > frame.start
                    && current_byte >= frame.marker_end
                    && current_byte <= range.end
                {
                    let end =
                        content_end_without_trailing_newline(source, frame.marker_end..range.end);
                    let empty = !frame.saw_content;
                    let at_end = current_byte == end;
                    return EnterContext::ListItem {
                        indent: frame.indent,
                        bullet: frame.bullet,
                        empty,
                        at_end,
                    };
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
                    if source_line_is_empty(source, current_byte) {
                        return EnterContext::EmptyBlockSeparatorLine;
                    }
                    return EnterContext::ParagraphInterior;
                }
            }
            Event::Start(Tag::CodeBlock(_)) => {
                mark_item_content_seen(&mut item_stack);
                code_block_range = Some(range.clone());
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(cb_range) = code_block_range.take()
                    && current_byte >= cb_range.start
                    && current_byte <= range.end
                {
                    return EnterContext::CodeBlock;
                }
            }
            Event::Start(Tag::Table(_)) => {
                mark_item_content_seen(&mut item_stack);
                table = Some(TableFrame { cell_ranges: Vec::new() });
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
            Event::Text(text) | Event::Code(text) | Event::Html(text) | Event::InlineHtml(text)
                if !text.is_empty() =>
            {
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
                        .map(|r| r.start);
                    return EnterContext::TableCell { next_cell_start };
                }
            }
        }
    }

    if let Some((line_start, content_start, line_end)) =
        locate_blockquote_line(source, current_byte)
        && let content_end = source_line_content_end(source, line_end)
        && current_byte >= content_start
        && current_byte <= content_end
    {
        let empty = content_start == content_end;
        return EnterContext::BlockQuoteLine {
            continuation_prefix: source[line_start..content_start].to_owned(),
            empty,
        };
    }

    if source_line_is_empty(source, current_byte) {
        return EnterContext::EmptyBlockSeparatorLine;
    }

    EnterContext::Other
}

fn source_line_is_empty(source: &str, byte: usize) -> bool {
    let Some((start, _, end)) = locate_source_line_bounds(source, byte) else {
        return false;
    };
    start == source_line_content_end(source, end)
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

// ─── 光标位置附近的源码切分 ───────────────────────────────────────────────

fn get_marker_delete_range(source: &str, current_byte: usize) -> Option<std::ops::Range<usize>> {
    let (line_start, _, _) = locate_source_line_bounds(source, current_byte)?;
    if line_start == current_byte {
        return None;
    }
    let prefix = &source[line_start..current_byte];
    let trimmed = prefix.trim_start_matches(' ');

    let is_match = match trimmed {
        _ if trimmed.starts_with('>') => trimmed.chars().all(|c| c == '>' || c == ' '),
        _ if trimmed.starts_with('#') => {
            let hashes = trimmed.chars().take_while(|&c| c == '#').count();
            hashes <= 6 && hashes > 0 && &trimmed[hashes..] == " "
        }
        _ if trimmed.starts_with('-') || trimmed.starts_with('*') || trimmed.starts_with('+') => {
            let rest = &trimmed[1..];
            rest == " " || rest == " [ ] " || rest == " [x] " || rest == " [X] "
        }
        _ => {
            let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
            digits > 0 && {
                let rest = &trimmed[digits..];
                rest == ". " || rest == ") "
            }
        }
    };

    if is_match { Some(line_start..current_byte) } else { None }
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
        let separator_start = marker_start + 1;
        if !matches!(bytes.get(separator_start), Some(b' ' | b'\t')) {
            return None;
        }

        let task_start = separator_start + 1;
        let Some(task_marker) = bytes.get(task_start..task_start + 3) else {
            return Some((ListBullet::Bullet, task_start));
        };
        let checked = match task_marker {
            b"[ ]" => false,
            b"[x]" | b"[X]" => true,
            _ => return Some((ListBullet::Bullet, task_start)),
        };
        let content_start = match bytes.get(task_start + 3) {
            Some(b' ' | b'\t') => task_start + 4,
            _ => task_start + 3,
        };
        return Some((ListBullet::TaskList(checked), content_start));
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
    if !(1..=9).contains(&digit_count)
        || !matches!(bytes.get(marker_end), Some(b'.' | b')'))
        || !matches!(bytes.get(marker_end + 1), Some(b' ' | b'\t'))
    {
        return None;
    }

    let number = source[marker_start..marker_end].parse::<u64>().ok()?;
    Some((ListBullet::Ordered(number), marker_end + 2))
}

pub(crate) fn list_item_indent(source: &str, marker_start: usize) -> String {
    let Some((line_start, _, _)) = locate_source_line_bounds(source, marker_start) else {
        return String::new();
    };
    source[line_start..marker_start].to_string()
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
    let mut marker_count = 0;
    while bytes.get(content_start) == Some(&b'>') {
        marker_count += 1;
        content_start += 1;
        if matches!(bytes.get(content_start), Some(b' ' | b'\t')) {
            content_start += 1;
        }
    }
    (marker_count > 0).then_some((line_start, content_start, line_end))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn parses_supported_list_marker_boundaries() {
        assert_eq!(parse_list_marker("42. item", 0), Some((ListBullet::Ordered(42), 4)));
        assert_eq!(parse_list_marker("-\titem", 0), Some((ListBullet::Bullet, 2)));
        assert_eq!(parse_list_marker("- [X] done", 0), Some((ListBullet::TaskList(true), 6)));
        assert_eq!(parse_list_marker("+ item", 0), Some((ListBullet::Bullet, 2)));
        assert_eq!(parse_list_marker("* item", 0), Some((ListBullet::Bullet, 2)));
        assert_eq!(parse_list_marker("7)\titem", 0), Some((ListBullet::Ordered(7), 3)));
        assert_eq!(parse_list_marker("- [ ] item", 0), Some((ListBullet::TaskList(false), 6)));
        assert_eq!(parse_list_marker("1234567890. item", 0), None);
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
    fn backspace_at_start_of_split_paragraph_restores_original_paragraph() {
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
    fn enter_then_backspace_restores_plain_paragraph_and_heading_cases() {
        let cases = [
            ("hello world", "hello ".len()),
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
        let cases = ["---\n\nparagraph", "- item\n\nparagraph", "```\ncode\n```\n\nparagraph"];

        for source in cases {
            let current_byte = source.find("paragraph").expect("fixture must contain a paragraph");

            assert!(
                augment_edit(source, current_byte, AugmentKind::Backspace).is_none(),
                "generic paragraph joining must not consume a {source:?} boundary"
            );
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
}
