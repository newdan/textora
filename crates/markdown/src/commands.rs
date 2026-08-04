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
    if selected.starts_with(marker) && selected.ends_with(marker) {
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
        let content = line[indentation..].trim_start_matches('#').trim_start();
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
    if segment.is_empty() {
        return SemanticEditPlan::NoChange;
    }
    let lines = segment.lines().collect::<Vec<_>>();
    let remove_existing = lines.iter().all(|line| has_line_prefix(line, prefix));
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
    if segment.is_empty() {
        return SemanticEditPlan::NoChange;
    }
    let fence = CODE_MARKER.repeat(3);
    let lines = segment.lines().collect::<Vec<_>>();
    let replacement = if lines.len() >= 2
        && lines.first().is_some_and(|line| line.trim_start().starts_with(&fence))
        && lines.last().is_some_and(|line| line.trim() == fence)
    {
        let inner_start = lines[0].len() + 1;
        let inner_end = segment.len().saturating_sub(lines.last().map_or(0, |line| line.len()) + 1);
        segment[inner_start..inner_end].to_owned()
    } else {
        format!("{fence}\n{segment}\n{fence}")
    };
    apply_line_transaction(source, source_generation, target, replacement)
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
    source[byte..].find('\n').map_or(source.len(), |newline| byte + newline)
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
    let mut result = String::with_capacity(segment.len());
    for line in segment.split_inclusive('\n') {
        let (content, newline) =
            line.strip_suffix('\n').map_or((line, ""), |content| (content, "\n"));
        result.push_str(&transform(content));
        result.push_str(newline);
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
}
