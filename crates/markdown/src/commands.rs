use ui::plugin::{SemanticEditCommand, SemanticEditPlan};

pub fn plan_semantic_edit(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
    command: SemanticEditCommand,
) -> SemanticEditPlan {
    if cursor_byte > source.len() || !source.is_char_boundary(cursor_byte) {
        return SemanticEditPlan::NoChange;
    }
    if selection.as_ref().is_some_and(|range| {
        range.start > range.end
            || range.end > source.len()
            || !source.is_char_boundary(range.start)
            || !source.is_char_boundary(range.end)
    }) {
        return SemanticEditPlan::NoChange;
    }

    match command {
        SemanticEditCommand::Undo | SemanticEditCommand::Redo => SemanticEditPlan::Unsupported,
        SemanticEditCommand::SetHeadingLevel(level) => {
            plan_heading(source, source_generation, cursor_byte, selection, level)
        }
        SemanticEditCommand::ToggleBold => {
            plan_inline_toggle(source, source_generation, cursor_byte, selection, "**")
        }
        SemanticEditCommand::ToggleItalic => {
            plan_inline_toggle(source, source_generation, cursor_byte, selection, "*")
        }
        SemanticEditCommand::ToggleStrikethrough => {
            plan_inline_toggle(source, source_generation, cursor_byte, selection, "~~")
        }
        SemanticEditCommand::ToggleInlineCode => {
            plan_inline_toggle(source, source_generation, cursor_byte, selection, CODE_MARKER)
        }
        SemanticEditCommand::UnorderedList => plan_line_prefix(
            source,
            source_generation,
            cursor_byte,
            selection,
            LinePrefix::Unordered,
        ),
        SemanticEditCommand::OrderedList => {
            plan_line_prefix(source, source_generation, cursor_byte, selection, LinePrefix::Ordered)
        }
        SemanticEditCommand::TaskList => {
            plan_line_prefix(source, source_generation, cursor_byte, selection, LinePrefix::Task)
        }
        SemanticEditCommand::Quote => {
            plan_line_prefix(source, source_generation, cursor_byte, selection, LinePrefix::Quote)
        }
        SemanticEditCommand::CodeBlock => {
            plan_code_block(source, source_generation, cursor_byte, selection)
        }
        SemanticEditCommand::InsertLink => {
            plan_link(source, source_generation, cursor_byte, selection)
        }
        SemanticEditCommand::PromoteObject | SemanticEditCommand::DemoteObject => {
            SemanticEditPlan::Unsupported
        }
    }
}

const CODE_MARKER: &str = "\x60";
const LF_SEQUENCE: &str = "\n";
const CRLF_SEQUENCE: &str = "\r\n";

#[derive(Clone, Copy)]
enum LinePrefix {
    Unordered,
    Ordered,
    Task,
    Quote,
}

fn plan_inline_toggle(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
    marker: &str,
) -> SemanticEditPlan {
    let Some(range) = selection.filter(|range| range.start < range.end) else {
        let replacement = format!("{marker}{marker}");
        return apply_transaction(
            source_generation,
            cursor_byte..cursor_byte,
            replacement,
            cursor_byte + marker.len(),
        );
    };
    let selected = &source[range.clone()];
    if selected.len() >= marker.len() * 2
        && selected.starts_with(marker)
        && selected.ends_with(marker)
    {
        let unwrapped = selected[marker.len()..selected.len() - marker.len()].to_owned();
        return apply_transaction(
            source_generation,
            range.clone(),
            unwrapped.clone(),
            range.start + unwrapped.len(),
        );
    }
    let replacement = format!("{marker}{selected}{marker}");
    apply_transaction(
        source_generation,
        range.clone(),
        replacement.clone(),
        range.start + replacement.len(),
    )
}

fn plan_link(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
) -> SemanticEditPlan {
    let Some(range) = selection.filter(|range| range.start < range.end) else {
        return apply_transaction(
            source_generation,
            cursor_byte..cursor_byte,
            "[](https://)".to_owned(),
            cursor_byte + 1,
        );
    };
    let replacement = format!("[{}](https://)", &source[range.clone()]);
    apply_transaction(
        source_generation,
        range.clone(),
        replacement.clone(),
        range.start + replacement.len(),
    )
}

fn plan_heading(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
    level: u8,
) -> SemanticEditPlan {
    if !(1..=6).contains(&level) {
        return SemanticEditPlan::Unsupported;
    }
    let target = line_selection_range(source, cursor_byte, selection.as_ref());
    if let Some(title_range) = first_h1_range(source)
        && ranges_overlap(&target, &title_range)
    {
        return SemanticEditPlan::NoChange;
    }
    let replacement = transform_lines(&source[target.clone()], |line| {
        let indentation = leading_whitespace(line);
        let content = &line[indentation..];
        let content = if crate::augmenter::heading_source_is_atx(line, 0) {
            content.trim_start_matches('#').trim_start_matches([' ', '\t'])
        } else {
            content
        };
        format!("{}{} {}", &line[..indentation], "#".repeat(level as usize), content)
    });
    apply_line_transaction(source, source_generation, target, replacement)
}

fn plan_line_prefix(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
    prefix: LinePrefix,
) -> SemanticEditPlan {
    let target = line_selection_range(source, cursor_byte, selection.as_ref());
    let segment = &source[target.clone()];
    let remove_existing = segment
        .split('\n')
        .all(|line| has_line_prefix(line.strip_suffix('\r').unwrap_or(line), prefix));
    let replacement = transform_lines(segment, |line| {
        if remove_existing {
            remove_line_prefix(line, prefix)
        } else {
            add_line_prefix(line, prefix)
        }
    });
    apply_line_transaction(source, source_generation, target, replacement)
}

fn plan_code_block(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
) -> SemanticEditPlan {
    let target = line_selection_range(source, cursor_byte, selection.as_ref());
    let segment = &source[target.clone()];
    let fence = CODE_MARKER.repeat(3);
    let replacement = unwrap_code_fence(segment, &fence).unwrap_or_else(|| {
        let newline = if source.find('\n').is_some_and(|index| source[..index].ends_with('\r')) {
            CRLF_SEQUENCE
        } else {
            LF_SEQUENCE
        };
        format!("{fence}{newline}{segment}{newline}{fence}")
    });
    apply_line_transaction(source, source_generation, target, replacement)
}

fn unwrap_code_fence(segment: &str, fence: &str) -> Option<String> {
    let (opening_line, after_opening) = segment.split_once('\n')?;
    if !opening_line.trim_start().starts_with(fence) {
        return None;
    }
    let through_closing = after_opening.trim_end_matches(['\r', '\n']);
    let closing_start = through_closing.rfind('\n').map_or(0, |newline| newline + 1);
    if through_closing[closing_start..].trim() != fence {
        return None;
    }
    let content = &after_opening[..closing_start];
    let content = content
        .strip_suffix(CRLF_SEQUENCE)
        .or_else(|| content.strip_suffix(LF_SEQUENCE))
        .unwrap_or(content);
    let after_closing = &after_opening[through_closing.len()..];
    Some(format!("{content}{after_closing}"))
}

fn apply_line_transaction(
    source: &str,
    source_generation: u32,
    range: std::ops::Range<usize>,
    replacement: String,
) -> SemanticEditPlan {
    if source[range.clone()] == replacement {
        return SemanticEditPlan::NoChange;
    }
    let cursor_after = range.start + replacement.len();
    apply_transaction(source_generation, range, replacement, cursor_after)
}

fn apply_transaction(
    source_generation: u32,
    range: std::ops::Range<usize>,
    replacement: String,
    cursor_after: usize,
) -> SemanticEditPlan {
    SemanticEditPlan::Apply(ui::plugin::EditTransaction::replace(
        source_generation,
        range,
        replacement,
        cursor_after,
    ))
}

fn line_selection_range(
    source: &str,
    cursor_byte: usize,
    selection: Option<&std::ops::Range<usize>>,
) -> std::ops::Range<usize> {
    let start_byte = selection.map_or(cursor_byte, |range| range.start);
    let end_byte = selection.map_or(cursor_byte, |range| range.end);
    let start = line_start(source, start_byte);
    let end_probe = if end_byte > start_byte && source.as_bytes().get(end_byte - 1) == Some(&b'\n')
    {
        end_byte - 1
    } else {
        end_byte
    };
    start..line_end(source, end_probe)
}

fn line_start(source: &str, byte: usize) -> usize {
    source[..byte].rfind('\n').map_or(0, |newline| newline + 1)
}

fn line_end(source: &str, byte: usize) -> usize {
    let Some(newline) = source[byte..].find('\n') else {
        return source.len();
    };
    let newline_byte = byte + newline;
    if source[..newline_byte].ends_with('\r') { newline_byte - 1 } else { newline_byte }
}

fn first_h1_range(source: &str) -> Option<std::ops::Range<usize>> {
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let indentation = leading_whitespace(content);
        if content[indentation..].starts_with("# ") {
            return Some(offset..offset + content.len());
        }
        offset += line.len();
    }
    None
}

fn ranges_overlap(left: &std::ops::Range<usize>, right: &std::ops::Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn leading_whitespace(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

fn transform_lines(segment: &str, transform: impl Fn(&str) -> String) -> String {
    if segment.is_empty() {
        return transform(segment);
    }
    let mut result = String::with_capacity(segment.len());
    for line in segment.split_inclusive('\n') {
        let (content, newline) = if let Some(content) = line.strip_suffix(CRLF_SEQUENCE) {
            (content, CRLF_SEQUENCE)
        } else {
            line.strip_suffix(LF_SEQUENCE).map_or((line, ""), |content| (content, LF_SEQUENCE))
        };
        result.push_str(&transform(content));
        result.push_str(newline);
    }
    if segment.ends_with(LF_SEQUENCE) {
        result.push_str(&transform(""));
    }
    result
}

fn has_line_prefix(line: &str, prefix: LinePrefix) -> bool {
    let content = &line[leading_whitespace(line)..];
    match prefix {
        LinePrefix::Unordered => content.starts_with("- ") || content.starts_with("* "),
        LinePrefix::Ordered => content.split_once(". ").is_some_and(|(number, _)| {
            !number.is_empty() && number.chars().all(|character| character.is_ascii_digit())
        }),
        LinePrefix::Task => {
            content.starts_with("- [ ] ")
                || content.starts_with("- [x] ")
                || content.starts_with("- [X] ")
        }
        LinePrefix::Quote => content.starts_with("> ") || content == ">",
    }
}

fn remove_line_prefix(line: &str, prefix: LinePrefix) -> String {
    let indentation = leading_whitespace(line);
    let content = &line[indentation..];
    let marker_length = match prefix {
        LinePrefix::Unordered if content.starts_with("- ") || content.starts_with("* ") => 2,
        LinePrefix::Ordered => content
            .find(". ")
            .filter(|index| {
                *index > 0 && content[..*index].chars().all(|character| character.is_ascii_digit())
            })
            .map_or(0, |index| index + 2),
        LinePrefix::Task
            if content.starts_with("- [ ] ")
                || content.starts_with("- [x] ")
                || content.starts_with("- [X] ") =>
        {
            6
        }
        LinePrefix::Quote if content.starts_with("> ") => 2,
        LinePrefix::Quote if content == ">" => 1,
        _ => 0,
    };
    format!("{}{}", &line[..indentation], &content[marker_length..])
}

fn add_line_prefix(line: &str, prefix: LinePrefix) -> String {
    let indentation = leading_whitespace(line);
    let marker = match prefix {
        LinePrefix::Unordered => "- ",
        LinePrefix::Ordered => "1. ",
        LinePrefix::Task => "- [ ] ",
        LinePrefix::Quote => "> ",
    };
    format!("{}{}{}", &line[..indentation], marker, &line[indentation..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn applied_text(
        source: &str,
        command: SemanticEditCommand,
        cursor_byte: usize,
        selection: Option<std::ops::Range<usize>>,
    ) -> (String, ui::plugin::EditSelection) {
        let plan = plan_semantic_edit(source, 7, cursor_byte, selection, command);
        let SemanticEditPlan::Apply(transaction) = plan else {
            panic!("semantic command should produce an edit transaction");
        };
        let replacement =
            transaction.replacements.first().expect("semantic command should have one replacement");
        let mut result = source.to_owned();
        result.replace_range(replacement.range.clone(), &replacement.text);
        (result, transaction.selection_after)
    }

    #[test]
    fn toggle_bold_without_selection_inserts_a_pair_and_places_the_caret_inside() {
        let (result, selection) =
            applied_text("标题", SemanticEditCommand::ToggleBold, "标题".len(), None);

        assert_eq!(result, "标题****");
        assert_eq!(selection, ui::plugin::EditSelection::Caret("标题".len() + 2));
    }

    #[test]
    fn toggle_bold_wraps_a_single_line_selection() {
        let (result, _) = applied_text("中文文本", SemanticEditCommand::ToggleBold, 6, Some(0..6));

        assert_eq!(result, "**中文**文本");
    }

    #[test]
    fn unordered_list_formats_every_line_in_a_multiline_selection() {
        let (result, _) = applied_text(
            "第一行\n第二行",
            SemanticEditCommand::UnorderedList,
            0,
            Some(0.."第一行\n第二行".len()),
        );

        assert_eq!(result, "- 第一行\n- 第二行");
    }

    #[test]
    fn toggling_bold_again_removes_existing_nested_markers() {
        let (result, _) = applied_text(
            "**重点**",
            SemanticEditCommand::ToggleBold,
            "**重点**".len(),
            Some(0.."**重点**".len()),
        );

        assert_eq!(result, "重点");
    }

    #[test]
    fn heading_command_does_not_demote_the_notora_title_h1() {
        let plan = plan_semantic_edit(
            "# 标题\n\n正文",
            7,
            2,
            Some(0..8),
            SemanticEditCommand::SetHeadingLevel(2),
        );

        assert_eq!(plan, SemanticEditPlan::NoChange);
    }

    #[test]
    fn inline_toggle_wraps_a_selection_that_contains_only_one_marker() {
        for (command, marker) in [
            (SemanticEditCommand::ToggleBold, "**"),
            (SemanticEditCommand::ToggleItalic, "*"),
            (SemanticEditCommand::ToggleStrikethrough, "~~"),
            (SemanticEditCommand::ToggleInlineCode, CODE_MARKER),
        ] {
            let (result, _) = applied_text(marker, command, marker.len(), Some(0..marker.len()));

            assert_eq!(result, marker.repeat(3));
        }
    }

    #[test]
    fn code_block_toggle_removes_an_empty_fenced_block() {
        let source = "```\n```";
        let (result, _) =
            applied_text(source, SemanticEditCommand::CodeBlock, 0, Some(0..source.len()));

        assert_eq!(result, "");
    }

    #[test]
    fn line_commands_format_an_empty_document() {
        for (command, expected) in [
            (SemanticEditCommand::SetHeadingLevel(2), "## "),
            (SemanticEditCommand::UnorderedList, "- "),
            (SemanticEditCommand::OrderedList, "1. "),
            (SemanticEditCommand::TaskList, "- [ ] "),
            (SemanticEditCommand::Quote, "> "),
            (SemanticEditCommand::CodeBlock, "```\n\n```"),
        ] {
            let (result, _) = applied_text("", command, 0, None);

            assert_eq!(result, expected);
        }
    }

    #[test]
    fn line_commands_format_a_blank_line_between_paragraphs() {
        for (command, expected) in [
            (SemanticEditCommand::SetHeadingLevel(2), "before\n## \nafter"),
            (SemanticEditCommand::UnorderedList, "before\n- \nafter"),
            (SemanticEditCommand::OrderedList, "before\n1. \nafter"),
            (SemanticEditCommand::TaskList, "before\n- [ ] \nafter"),
            (SemanticEditCommand::Quote, "before\n> \nafter"),
            (SemanticEditCommand::CodeBlock, "before\n```\n\n```\nafter"),
        ] {
            let (result, _) = applied_text("before\n\nafter", command, "before\n".len(), None);

            assert_eq!(result, expected);
        }
    }

    #[test]
    fn heading_command_preserves_hashes_that_are_part_of_plain_text() {
        for source in ["#hashtag", "##正文", "####### seven hashes"] {
            let (result, _) =
                applied_text(source, SemanticEditCommand::SetHeadingLevel(2), 0, None);

            assert_eq!(result, format!("## {source}"));
        }
    }

    #[test]
    fn line_command_keeps_the_crlf_sequence_outside_its_replacement_range() {
        let plan = plan_semantic_edit("first\r\nsecond", 7, 0, None, SemanticEditCommand::Quote);
        let SemanticEditPlan::Apply(transaction) = plan else {
            panic!("quote command should produce an edit transaction");
        };

        assert_eq!(transaction.replacements[0].range, 0.."first".len());
        assert_eq!(transaction.replacements[0].text, "> first");
    }

    #[test]
    fn code_block_toggle_preserves_crlf_when_adding_fences() {
        let source = "first\r\nsecond";
        let (result, _) =
            applied_text(source, SemanticEditCommand::CodeBlock, 0, Some(0..source.len()));

        assert_eq!(result, "```\r\nfirst\r\nsecond\r\n```");
    }

    #[test]
    fn code_block_toggle_preserves_crlf_when_removing_fences() {
        let source = "```rust\r\nfirst\r\nsecond\r\n```\r\nafter";
        let closing_end =
            source.find("\r\nafter").expect("fixture has a paragraph after the fence");
        let (result, _) =
            applied_text(source, SemanticEditCommand::CodeBlock, 0, Some(0..closing_end));

        assert_eq!(result, "first\r\nsecond\r\nafter");
    }

    #[test]
    fn code_block_toggle_preserves_a_selected_blank_line_after_the_closing_fence() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("```{newline}a{newline}```{newline}{newline}last");
            let selection_end = source.find("last").expect("fixture ends with a paragraph");
            let (result, _) =
                applied_text(&source, SemanticEditCommand::CodeBlock, 0, Some(0..selection_end));

            assert_eq!(result, format!("a{newline}{newline}last"));
        }
    }

    #[test]
    fn empty_code_block_toggle_preserves_a_selected_blank_line_after_the_closing_fence() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("```{newline}```{newline}{newline}last");
            let selection_end = source.find("last").expect("fixture ends with a paragraph");
            let (result, _) =
                applied_text(&source, SemanticEditCommand::CodeBlock, 0, Some(0..selection_end));

            assert_eq!(result, format!("{newline}{newline}last"));
        }
    }

    #[test]
    fn code_block_toggle_preserves_multiple_selected_blank_lines_after_the_closing_fence() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("```{newline}a{newline}```{newline}{newline}{newline}last");
            let selection_end = source.find("last").expect("fixture ends with a paragraph");
            let (result, _) =
                applied_text(&source, SemanticEditCommand::CodeBlock, 0, Some(0..selection_end));

            assert_eq!(result, format!("a{newline}{newline}{newline}last"));
        }
    }

    #[test]
    fn multiline_heading_selection_formats_its_last_blank_line() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("first{newline}{newline}last");
            let selection_end = format!("first{newline}{newline}").len();
            let (result, _) = applied_text(
                &source,
                SemanticEditCommand::SetHeadingLevel(2),
                0,
                Some(0..selection_end),
            );

            assert_eq!(result, format!("## first{newline}## {newline}last"));
        }
    }

    #[test]
    fn multiline_quote_selection_counts_its_last_blank_line_when_toggling() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("> first{newline}{newline}last");
            let selection_end = format!("> first{newline}{newline}").len();
            let (result, _) =
                applied_text(&source, SemanticEditCommand::Quote, 0, Some(0..selection_end));

            assert_eq!(result, format!("> > first{newline}> {newline}last"));
        }
    }
}
