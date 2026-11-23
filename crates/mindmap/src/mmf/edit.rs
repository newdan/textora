use crate::mmf::model::{GlobalPropertySource, Node, Tree};
use crate::mmf::utils::{collect_nodes_dfs, find_siblings};
use ui::plugin::{
    EditIntent, EditPlan, EditRequest, EditSelection, EditTransaction, TextReplacement,
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MoveSubtreeTarget {
    BeforeSibling,
    AfterSibling,
    BeforeChild,
    LastChild,
}

/// 将思维导图的语义编辑请求转换为局部源码事务。
///
/// 此函数只读取解析树、源码和请求；文档写入由 app 层统一执行事务。
pub fn plan_mindmap_edit(tree: &Tree, source: &str, request: &EditRequest) -> EditPlan {
    let Some(node) = focused_node(tree, request) else {
        return EditPlan::Consume;
    };
    let node_index = node_index(tree, node).expect("focused node always belongs to its tree");
    let node_selected = request.selection.as_ref() == Some(&node.subtree_source_range);

    match &request.intent {
        EditIntent::InsertText(text) if node_selected => EditPlan::Apply(EditTransaction::replace(
            request.source_generation,
            node.title_byte_range.clone(),
            text.clone(),
            node.title_byte_range.start + text.len(),
        )),
        EditIntent::InsertText(text) => plan_title_text_insertion(source, request, node, text),
        EditIntent::InsertParagraphBreak if node_selected => {
            EditPlan::SetSelection(EditSelection::Caret(node.title_byte_range.end))
        }
        EditIntent::InsertParagraphBreak => {
            if node_index == 0 {
                EditPlan::Consume
            } else {
                plan_new_sibling(source, request, node)
            }
        }
        EditIntent::InsertLineBreak => EditPlan::Consume,
        EditIntent::Indent => plan_new_child(source, request, node),
        EditIntent::Outdent | EditIntent::PromoteObject => {
            if node_index == 0 {
                EditPlan::Consume
            } else {
                plan_promote_subtree(request, node)
            }
        }
        EditIntent::DemoteObject => {
            if node_index == 0 || !has_previous_sibling(tree, node_index) {
                EditPlan::Consume
            } else {
                plan_demote_subtree(request, node)
            }
        }
        EditIntent::DeleteBackward | EditIntent::DeleteForward if node_selected => {
            if node_index == 0 {
                EditPlan::Consume
            } else {
                plan_delete_subtree(tree, request, node)
            }
        }
        EditIntent::DeleteBackward => {
            plan_title_deletion(source, request, node, DeleteDirection::Backward)
        }
        EditIntent::DeleteForward => {
            plan_title_deletion(source, request, node, DeleteDirection::Forward)
        }
        EditIntent::SelectObject if node_selected => EditPlan::Consume,
        EditIntent::SelectObject => EditPlan::SetSelection(EditSelection::Range {
            anchor: node.subtree_source_range.start,
            cursor: node.subtree_source_range.end,
        }),
    }
}

/// 在当前节点子树之后插入同级空标题。
pub fn plan_new_sibling(source: &str, request: &EditRequest, node: &Node) -> EditPlan {
    let insertion_byte = node.subtree_source_range.end;
    let (text, cursor_after) = empty_heading_insertion(source, insertion_byte, node.heading_level);
    insertion_plan(request, insertion_byte, text, cursor_after)
}

/// 在节点自身内容结束、首个直接子节点之前插入空标题。
pub fn plan_new_child(source: &str, request: &EditRequest, node: &Node) -> EditPlan {
    let insertion_byte = node.child_insertion_byte;
    let (text, cursor_after) =
        empty_heading_insertion(source, insertion_byte, node.heading_level.saturating_add(1));
    insertion_plan(request, insertion_byte, text, cursor_after)
}

/// 将当前节点和所有后代提升一级，只删除解析器确认的标题 marker。
pub fn plan_promote_subtree(request: &EditRequest, node: &Node) -> EditPlan {
    let nodes = collect_nodes_dfs(node);
    let replacements = nodes
        .iter()
        .map(|descendant| TextReplacement {
            range: descendant.heading_marker_range.end - 1..descendant.heading_marker_range.end,
            text: String::new(),
        })
        .collect::<Vec<_>>();
    let selection_after = level_change_selection(node, request, -(nodes.len() as isize));

    EditPlan::Apply(EditTransaction {
        source_generation: request.source_generation,
        replacements,
        selection_after,
    })
}

/// 将当前节点和所有后代降级一级，只修改解析器确认的标题 marker。
pub fn plan_demote_subtree(request: &EditRequest, node: &Node) -> EditPlan {
    let nodes = collect_nodes_dfs(node);
    let replacements = nodes
        .iter()
        .map(|descendant| TextReplacement {
            range: descendant.heading_marker_range.end..descendant.heading_marker_range.end,
            text: "#".into(),
        })
        .collect::<Vec<_>>();
    let selection_after = level_change_selection(node, request, nodes.len() as isize);

    EditPlan::Apply(EditTransaction {
        source_generation: request.source_generation,
        replacements,
        selection_after,
    })
}

/// 删除当前节点及其后代，并把选择恢复到相邻的可见对象。
pub fn plan_delete_subtree(tree: &Tree, request: &EditRequest, node: &Node) -> EditPlan {
    let delete_range = subtree_delete_range(node);
    EditPlan::Apply(EditTransaction {
        source_generation: request.source_generation,
        replacements: vec![TextReplacement { range: delete_range.clone(), text: String::new() }],
        selection_after: selection_after_subtree_deletion(tree, node, &delete_range),
    })
}

/// 计划切换非根节点的持久化折叠状态。
pub(crate) fn plan_toggle_collapsed(
    tree: &Tree,
    source: &str,
    node_range: std::ops::Range<usize>,
    source_generation: u32,
) -> EditPlan {
    let nodes = collect_nodes_dfs(&tree.root);
    let Some(node_index) = nodes.iter().position(|node| node.subtree_source_range == node_range)
    else {
        return EditPlan::Consume;
    };
    if node_index == 0 {
        return EditPlan::Consume;
    }
    let node = nodes[node_index];
    if node.children.is_empty() {
        return EditPlan::Consume;
    }

    let newline = document_newline(source);
    let Some(replacement) = (match (&node.property_source, node.props.as_ref()) {
        (Some(property_source), Some(props)) => {
            let range = property_source
                .collapsed_value_range
                .clone()
                .unwrap_or(property_source.body_range.end..property_source.body_range.end);
            let text = if range.is_empty() {
                format!("collapsed = {}{newline}", !props.collapsed)
            } else {
                (!props.collapsed).to_string()
            };
            Some(TextReplacement { range, text })
        }
        (None, _) => Some(TextReplacement {
            range: node.heading_source_end..node.heading_source_end,
            text: format!("{newline}```toml node{newline}collapsed = true{newline}```{newline}"),
        }),
        (Some(_), None) => None,
    }) else {
        return EditPlan::Consume;
    };

    EditPlan::Apply(EditTransaction {
        source_generation,
        replacements: vec![replacement],
        selection_after: EditSelection::Caret(node.title_byte_range.end),
    })
}

/// 计划设置思维导图全局主题。
pub fn plan_set_mindmap_theme(
    tree: &Tree,
    source: &str,
    theme_id: &str,
    source_generation: u32,
    cursor_byte: usize,
) -> EditPlan {
    let current_theme = tree.global_props.get("theme").map(String::as_str);
    if current_theme.unwrap_or(ui::theme::DEFAULT_MINDMAP_COLOR_SCHEME_ID) == theme_id {
        return EditPlan::Consume;
    }
    match crate::mmf::parser::parse_global_property_source(source) {
        Err(_) => EditPlan::Consume,
        Ok(Some(property_source)) => plan_existing_global_block(
            source,
            property_source,
            theme_id,
            source_generation,
            cursor_byte,
        ),
        Ok(None) => plan_new_global_block(source, tree, theme_id, source_generation, cursor_byte),
    }
}

fn plan_existing_global_block(
    source: &str,
    property_source: GlobalPropertySource,
    theme_id: &str,
    source_generation: u32,
    cursor_byte: usize,
) -> EditPlan {
    let newline = document_newline(source);
    let (range, text) = match property_source.theme_value_range {
        Some(value_range) => (value_range, format!("\"{theme_id}\"")),
        None => (
            property_source.body_range.end..property_source.body_range.end,
            format!("theme = \"{theme_id}\"{newline}"),
        ),
    };
    EditPlan::Apply(EditTransaction {
        source_generation,
        replacements: vec![TextReplacement { range: range.clone(), text: text.clone() }],
        selection_after: EditSelection::Caret(adjust_cursor_for_replacement(
            cursor_byte,
            &range,
            text.len(),
        )),
    })
}

fn plan_new_global_block(
    source: &str,
    tree: &Tree,
    theme_id: &str,
    source_generation: u32,
    cursor_byte: usize,
) -> EditPlan {
    let newline = document_newline(source);
    let insertion_byte = tree.root.source_range.start;
    let text = format!(
        "```toml mindmap{newline}version = 1{newline}theme = \"{theme_id}\"{newline}```{newline}{newline}"
    );
    let range = insertion_byte..insertion_byte;
    EditPlan::Apply(EditTransaction {
        source_generation,
        replacements: vec![TextReplacement { range: range.clone(), text: text.clone() }],
        selection_after: EditSelection::Caret(adjust_cursor_for_replacement(
            cursor_byte,
            &range,
            text.len(),
        )),
    })
}

fn adjust_cursor_for_replacement(
    cursor_byte: usize,
    replacement_range: &std::ops::Range<usize>,
    new_text_len: usize,
) -> usize {
    if cursor_byte <= replacement_range.start {
        cursor_byte
    } else if cursor_byte <= replacement_range.end {
        replacement_range.start + new_text_len
    } else {
        let original_len = replacement_range.end - replacement_range.start;
        cursor_byte + new_text_len - original_len
    }
}

/// 计划在一个局部源码事务中移动 MMF 子树。
pub(crate) fn plan_move_subtree(
    tree: &Tree,
    source: &str,
    source_range: std::ops::Range<usize>,
    anchor_range: std::ops::Range<usize>,
    target: MoveSubtreeTarget,
    source_generation: u32,
) -> EditPlan {
    let nodes = collect_nodes_dfs(&tree.root);
    let Some(source_index) =
        nodes.iter().position(|node| node.subtree_source_range == source_range)
    else {
        return EditPlan::Consume;
    };
    if source_index == 0 {
        return EditPlan::Consume;
    }
    let Some(anchor_index) =
        nodes.iter().position(|node| node.subtree_source_range == anchor_range)
    else {
        return EditPlan::Consume;
    };

    let source_node = nodes[source_index];
    let anchor_node = nodes[anchor_index];
    if collect_nodes_dfs(source_node).into_iter().any(|node| std::ptr::eq(node, anchor_node)) {
        return EditPlan::Consume;
    }

    let sibling_offset = match target {
        MoveSubtreeTarget::BeforeSibling | MoveSubtreeTarget::AfterSibling if anchor_index != 0 => {
            0
        }
        MoveSubtreeTarget::BeforeSibling | MoveSubtreeTarget::AfterSibling => {
            return EditPlan::Consume;
        }
        MoveSubtreeTarget::BeforeChild => 0,
        MoveSubtreeTarget::LastChild => 1,
    };
    let Some(target_heading_level) = anchor_node.heading_level.checked_add(sibling_offset) else {
        return EditPlan::Consume;
    };
    let Some(releveled_headings) = releveled_heading_levels(source_node, target_heading_level)
    else {
        return EditPlan::Consume;
    };
    let Some(source_block) = source.get(source_node.subtree_source_range.clone()) else {
        return EditPlan::Consume;
    };
    let moved_block = relevel_subtree_source(
        source_block,
        source_node.subtree_source_range.start,
        &releveled_headings,
    );
    let insertion_byte = match target {
        MoveSubtreeTarget::BeforeSibling | MoveSubtreeTarget::BeforeChild => {
            anchor_node.subtree_source_range.start
        }
        MoveSubtreeTarget::AfterSibling | MoveSubtreeTarget::LastChild => {
            anchor_node.subtree_source_range.end
        }
    };
    if target == MoveSubtreeTarget::AfterSibling
        && insertion_byte == source_node.subtree_source_range.start
    {
        return EditPlan::SetSelection(EditSelection::Range {
            anchor: source_node.subtree_source_range.start,
            cursor: source_node.subtree_source_range.end,
        });
    }
    let bounded_block = ensure_block_boundaries(source, insertion_byte, moved_block);
    let selection_len = bounded_block.text.len() - bounded_block.prefix_len;
    if insertion_byte == source_node.subtree_source_range.start {
        let selection_start = source_node.subtree_source_range.start + bounded_block.prefix_len;
        return EditPlan::Apply(EditTransaction {
            source_generation,
            replacements: vec![TextReplacement {
                range: source_node.subtree_source_range.clone(),
                text: bounded_block.text,
            }],
            selection_after: EditSelection::Range {
                anchor: selection_start,
                cursor: selection_start + selection_len,
            },
        });
    }
    let selection_start = if insertion_byte < source_node.subtree_source_range.start {
        insertion_byte
    } else {
        insertion_byte - source_node.subtree_source_range.len()
    } + bounded_block.prefix_len;

    EditPlan::Apply(EditTransaction {
        source_generation,
        replacements: vec![
            TextReplacement {
                range: source_node.subtree_source_range.clone(),
                text: String::new(),
            },
            TextReplacement { range: insertion_byte..insertion_byte, text: bounded_block.text },
        ],
        selection_after: EditSelection::Range {
            anchor: selection_start,
            cursor: selection_start + selection_len,
        },
    })
}

fn releveled_heading_levels(
    source_node: &Node,
    target_heading_level: u8,
) -> Option<Vec<(&Node, u8)>> {
    collect_nodes_dfs(source_node)
        .into_iter()
        .map(|node| {
            let relative_depth = node.heading_level.checked_sub(source_node.heading_level)?;
            let releveled_heading = target_heading_level.checked_add(relative_depth)?;
            Some((node, releveled_heading))
        })
        .collect()
}

fn relevel_subtree_source(
    source_block: &str,
    source_start: usize,
    releveled_headings: &[(&Node, u8)],
) -> String {
    let mut moved_block = source_block.to_string();
    for (node, heading_level) in releveled_headings.iter().rev() {
        let relative_marker_range = node.heading_marker_range.start - source_start
            ..node.heading_marker_range.end - source_start;
        let target_marker = "#".repeat(usize::from(*heading_level));
        moved_block.replace_range(relative_marker_range, &target_marker);
    }
    moved_block
}

struct BoundaryAdjustedBlock {
    text: String,
    prefix_len: usize,
}

fn ensure_block_boundaries(
    source: &str,
    insertion_byte: usize,
    mut moved_block: String,
) -> BoundaryAdjustedBlock {
    let newline = document_newline(source);
    let needs_prefix = insertion_byte > 0
        && source.get(..insertion_byte).is_some_and(|prefix| !prefix.ends_with('\n'));
    let needs_suffix = insertion_byte < source.len()
        && source.get(insertion_byte..).is_some_and(|suffix| !suffix.starts_with('\n'))
        && !moved_block.ends_with('\n');
    let prefix_len = if needs_prefix { newline.len() } else { 0 };
    if needs_prefix {
        moved_block.insert_str(0, newline);
    }
    if needs_suffix {
        moved_block.push_str(newline);
    }
    BoundaryAdjustedBlock { text: moved_block, prefix_len }
}

fn focused_node<'a>(tree: &'a Tree, request: &EditRequest) -> Option<&'a Node> {
    let nodes = collect_nodes_dfs(&tree.root);
    if let Some(selection) = &request.selection {
        if let Some(node) = nodes.iter().find(|node| node.subtree_source_range == *selection) {
            return Some(*node);
        }
        return nodes.into_iter().find(|node| {
            selection.start >= node.title_byte_range.start
                && selection.end <= node.title_byte_range.end
        });
    }
    nodes.into_iter().find(|node| {
        request.cursor_byte >= node.title_byte_range.start
            && request.cursor_byte <= node.title_byte_range.end
    })
}

fn node_index(tree: &Tree, target: &Node) -> Option<usize> {
    collect_nodes_dfs(&tree.root).iter().position(|node| std::ptr::eq(*node, target))
}

fn has_previous_sibling(tree: &Tree, node_index: usize) -> bool {
    let Some(siblings) = find_siblings(tree, node_index) else {
        return false;
    };
    siblings.first().copied() != Some(node_index)
}

fn plan_title_text_insertion(
    source: &str,
    request: &EditRequest,
    node: &Node,
    text: &str,
) -> EditPlan {
    let range = match title_selection_range(request, node) {
        Some(range) => range,
        None if is_title_cursor(source, request.cursor_byte, node) => {
            request.cursor_byte..request.cursor_byte
        }
        None => return EditPlan::Consume,
    };
    EditPlan::Apply(EditTransaction::replace(
        request.source_generation,
        range.clone(),
        text.into(),
        range.start + text.len(),
    ))
}

enum DeleteDirection {
    Backward,
    Forward,
}

fn plan_title_deletion(
    source: &str,
    request: &EditRequest,
    node: &Node,
    direction: DeleteDirection,
) -> EditPlan {
    let range = match title_selection_range(request, node) {
        Some(range) => Some(range),
        None => match direction {
            DeleteDirection::Backward => {
                previous_title_character_range(source, request.cursor_byte, node)
            }
            DeleteDirection::Forward => {
                next_title_character_range(source, request.cursor_byte, node)
            }
        },
    };
    let Some(range) = range else {
        return EditPlan::Consume;
    };
    if range.is_empty() {
        return EditPlan::Consume;
    }

    EditPlan::Apply(EditTransaction::replace(
        request.source_generation,
        range.clone(),
        String::new(),
        range.start,
    ))
}

fn title_selection_range(request: &EditRequest, node: &Node) -> Option<std::ops::Range<usize>> {
    let selection = request.selection.as_ref()?;
    (selection.start >= node.title_byte_range.start && selection.end <= node.title_byte_range.end)
        .then(|| selection.clone())
}

fn is_title_cursor(source: &str, cursor_byte: usize, node: &Node) -> bool {
    cursor_byte >= node.title_byte_range.start
        && cursor_byte <= node.title_byte_range.end
        && source.is_char_boundary(cursor_byte)
}

fn previous_title_character_range(
    source: &str,
    cursor_byte: usize,
    node: &Node,
) -> Option<std::ops::Range<usize>> {
    if !is_title_cursor(source, cursor_byte, node) || cursor_byte == node.title_byte_range.start {
        return None;
    }
    let title_prefix = source.get(node.title_byte_range.start..cursor_byte)?;
    let (cluster_start, cluster) = title_prefix.grapheme_indices(true).next_back()?;
    Some(
        node.title_byte_range.start + cluster_start
            ..node.title_byte_range.start + cluster_start + cluster.len(),
    )
}

fn next_title_character_range(
    source: &str,
    cursor_byte: usize,
    node: &Node,
) -> Option<std::ops::Range<usize>> {
    if !is_title_cursor(source, cursor_byte, node) || cursor_byte == node.title_byte_range.end {
        return None;
    }
    let title_suffix = source.get(cursor_byte..node.title_byte_range.end)?;
    let (_, cluster) = title_suffix.grapheme_indices(true).next()?;
    Some(cursor_byte..cursor_byte + cluster.len())
}

fn empty_heading_insertion(
    source: &str,
    insertion_byte: usize,
    heading_level: u8,
) -> (String, usize) {
    let newline = document_newline(source);
    let prefix = if source
        .get(..insertion_byte)
        .is_some_and(|before_insertion| before_insertion.ends_with('\n'))
    {
        ""
    } else {
        newline
    };
    let marker = "#".repeat(heading_level as usize);
    let cursor_after = insertion_byte + prefix.len() + marker.len();
    (format!("{prefix}{marker}{newline}"), cursor_after)
}

fn document_newline(source: &str) -> &'static str {
    if source.contains("\r\n") { "\r\n" } else { "\n" }
}

fn insertion_plan(
    request: &EditRequest,
    insertion_byte: usize,
    text: String,
    cursor_after: usize,
) -> EditPlan {
    EditPlan::Apply(EditTransaction::replace(
        request.source_generation,
        insertion_byte..insertion_byte,
        text,
        cursor_after,
    ))
}

fn subtree_delete_range(node: &Node) -> std::ops::Range<usize> {
    node.subtree_source_range.clone()
}

fn selection_after_subtree_deletion(
    tree: &Tree,
    deleted_node: &Node,
    delete_range: &std::ops::Range<usize>,
) -> EditSelection {
    let nodes = collect_nodes_dfs(&tree.root);
    let Some(deleted_index) = nodes.iter().position(|node| std::ptr::eq(*node, deleted_node))
    else {
        return EditSelection::Caret(delete_range.start);
    };

    let target = deleted_index
        .checked_sub(1)
        .and_then(|index| nodes.get(index).copied())
        .or_else(|| crate::mmf::utils::find_parent(tree, deleted_index))
        .or_else(|| nodes.get(deleted_index + 1).copied());

    target
        .map(|node| selected_object_after_deletion(node, delete_range))
        .unwrap_or(EditSelection::Caret(delete_range.start))
}

fn selected_object_after_deletion(
    node: &Node,
    delete_range: &std::ops::Range<usize>,
) -> EditSelection {
    EditSelection::Range {
        anchor: byte_after_deletion(node.subtree_source_range.start, delete_range),
        cursor: byte_after_deletion(node.subtree_source_range.end, delete_range),
    }
}

fn byte_after_deletion(byte: usize, delete_range: &std::ops::Range<usize>) -> usize {
    if byte <= delete_range.start {
        return byte;
    }
    if byte >= delete_range.end {
        return byte - delete_range.len();
    }
    delete_range.start
}

fn level_change_selection(node: &Node, request: &EditRequest, delta: isize) -> EditSelection {
    if request.selection.as_ref() == Some(&node.subtree_source_range) {
        return EditSelection::Range {
            anchor: node.subtree_source_range.start,
            cursor: offset_by(node.subtree_source_range.end, delta),
        };
    }
    if let Some(selection) = title_selection_range(request, node) {
        return EditSelection::Range {
            anchor: offset_by(selection.start, delta.signum()),
            cursor: offset_by(selection.end, delta.signum()),
        };
    }
    EditSelection::Caret(offset_by(request.cursor_byte, delta.signum()))
}

fn offset_by(byte: usize, delta: isize) -> usize {
    if delta.is_negative() {
        byte.saturating_sub(delta.unsigned_abs())
    } else {
        byte.saturating_add(delta as usize)
    }
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;
