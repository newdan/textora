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
    if line_start != source_line_content_end(source, line_end) {
        return None;
    }

    let previous_line_end = previous_non_empty_line_end(source, current_byte);
    let (delete_range, cursor_byte_after) = if current_byte >= source.len() {
        let previous_newline_width = newline_sequence_width_before(source, current_byte)?;
        if contiguous_newline_count_before(source, current_byte) > BLOCK_BOUNDARY_NEWLINE_COUNT {
            let range = current_byte - previous_newline_width..current_byte;
            let cursor = range.start;
            (range, cursor)
        } else {
            (previous_line_end..current_byte, previous_line_end)
        }
    } else {
        let newline_width = newline_sequence_width_at(source, line_start)?;
        (line_start..line_start + newline_width, previous_line_end)
    };
    let aug = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(delete_range),
        cursor_byte_after,
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
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

    match classify_enter_context(source, boundary_start) {
        EnterContext::TopLevelParagraphEnd
        | EnterContext::ParagraphInterior
        | EnterContext::Heading { .. } => {
            let aug = EditAugmentation {
                insert_text: Some(String::new()),
                replace_range: Some(boundary_start..current_byte),
                cursor_byte_after: boundary_start,
            };
            debug_assert_augmentation(&aug, source);
            Some(aug)
        }
        EnterContext::BlockQuoteLine { continuation_prefix, .. } => Some(
            merge_into_preceding_block(source, boundary_start, current_byte, &continuation_prefix),
        ),
        EnterContext::ListItem { indent, continuation_marker, .. } => {
            // 并入为同一 item 的延续行：缩进 = item 前导空白 + marker 宽度的空格，
            // 对齐 item 的内容列。marker 均为 ASCII，但按 char 计宽更严谨。
            let marker_column_width = continuation_marker.chars().count();
            let content_padding = format!("{indent}{}", " ".repeat(marker_column_width));
            Some(merge_into_preceding_block(source, boundary_start, current_byte, &content_padding))
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

fn previous_non_empty_line_end(source: &str, current_byte: usize) -> usize {
    let mut previous_line_end = current_byte.min(source.len());
    while let Some(newline_width) = newline_sequence_width_before(source, previous_line_end) {
        previous_line_end -= newline_width;
    }
    previous_line_end
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
        EnterContext::ListItem { indent, continuation_marker, empty, at_end: _ } => {
            list_item_enter_augmentation(source, current_byte, &indent, &continuation_marker, empty)
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
    continuation_marker: &str,
    empty: bool,
) -> Option<EditAugmentation> {
    if empty {
        return emit_remove_current_line(source, current_byte);
    }
    Some(emit_marker_break(source, current_byte, indent, continuation_marker))
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
    ListItem { indent: String, continuation_marker: String, empty: bool, at_end: bool },
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
    continuation_marker: String,
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
    // pulldown-cmark 对 ATX 与 setext 标题都发 `Tag::Heading`，但下面的
    // `hash_prefix` 计算只适用于 ATX；setext（`Title\n===`）必须排除，
    // 否则回车/退格会按 `# ` 前缀语义破坏标题。
    if !heading_source_is_atx(source, start) {
        return EnterContext::Other;
    }
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

/// 标题 range 起始处的源码必须是合法的 ATX marker：至多 3 个前导空格，
/// 1-6 个 `#`，后跟空格/制表符/行尾。
fn heading_source_is_atx(source: &str, heading_start: usize) -> bool {
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
                        continuation_marker: list_continuation_marker(
                            source,
                            range.start,
                            marker_end,
                            bullet,
                        ),
                        saw_content: false,
                    });
                } else {
                    item_stack.push(ItemFrame {
                        start: range.start,
                        marker_end: range.start,
                        indent,
                        continuation_marker: String::from("- "),
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
                    if frame.marker_end > frame.start
                        && current_byte >= frame.marker_end
                        && current_byte <= end
                    {
                        let empty = !frame.saw_content;
                        let at_end = current_byte == end;
                        return EnterContext::ListItem {
                            indent: frame.indent,
                            continuation_marker: frame.continuation_marker,
                            empty,
                            at_end,
                        };
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
                if let Some(cb_range) = code_block_range.take() {
                    // pulldown-cmark 对缩进代码块的 range 包含结尾换行
                    // （fenced 则不包含）；用去尾换行的 content_end 统一两种边界，
                    // 否则紧邻的下一块首字节会被误判为 CodeBlock。
                    let end =
                        content_end_without_trailing_newline(source, cb_range.start..range.end);
                    if current_byte >= cb_range.start && current_byte <= end {
                        return EnterContext::CodeBlock;
                    }
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
        _ if trimmed.starts_with('>') => {
            trimmed.chars().all(|character| matches!(character, '>' | ' ' | '\t'))
        }
        _ if trimmed.starts_with('#') => {
            let hashes = trimmed.chars().take_while(|&c| c == '#').count();
            hashes <= 6 && hashes > 0 && is_single_marker_separator(&trimmed[hashes..])
        }
        _ if trimmed.starts_with('-') || trimmed.starts_with('*') || trimmed.starts_with('+') => {
            unordered_marker_suffix_is_complete(&trimmed[1..])
        }
        _ => {
            let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
            digits > 0 && {
                let rest = &trimmed[digits..];
                rest.strip_prefix(['.', ')']).is_some_and(is_single_marker_separator)
            }
        }
    };

    if is_match { Some(line_start..current_byte) } else { None }
}

fn is_single_marker_separator(text: &str) -> bool {
    matches!(text.as_bytes(), [b' '] | [b'\t'])
}

fn unordered_marker_suffix_is_complete(suffix: &str) -> bool {
    let Some((&separator, after_separator)) = suffix.as_bytes().split_first() else {
        return false;
    };
    if !matches!(separator, b' ' | b'\t') {
        return false;
    }
    if after_separator.is_empty() {
        return true;
    }

    let task_suffix = std::str::from_utf8(after_separator)
        .expect("a suffix sliced from valid UTF-8 source must remain valid UTF-8");
    ["[ ]", "[x]", "[X]"].into_iter().any(|task_marker| {
        task_suffix
            .strip_prefix(task_marker)
            .is_some_and(|trailing| trailing.is_empty() || is_single_marker_separator(trailing))
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
        ListBullet::TaskList(false) => source_marker.to_owned(),
        ListBullet::TaskList(true) => {
            source_marker.replacen("[x]", "[ ]", 1).replacen("[X]", "[ ]", 1)
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
    fn list_enter_preserves_marker_style_and_resets_completed_tasks() {
        let cases = [
            ("* item", "\n* "),
            ("+ item", "\n+ "),
            ("7) item", "\n8) "),
            ("-\titem", "\n-\t"),
            ("- [x] done", "\n- [ ] "),
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
    fn backspace_removes_markers_that_use_tab_separators() {
        for source in ["-\t", ">\t", "1.\t"] {
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
    fn setext_heading_is_not_classified_as_atx_heading() {
        let source = "Title\n===";

        assert!(
            matches!(classify_enter_context(source, "Title".len()), EnterContext::Other),
            "setext title text must not use ATX heading semantics"
        );
        assert!(
            matches!(classify_enter_context(source, source.len()), EnterContext::Other),
            "setext underline must not use ATX heading semantics"
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
}
