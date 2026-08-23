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
    match crate::mmf::parser::parse_global_property_source(source) {
        Err(_) => EditPlan::Consume,
        Ok(Some(_)) if tree.global_props.get("theme").is_some_and(|id| id == theme_id) => {
            EditPlan::Consume
        }
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
mod tests {
    use std::ops::Range;

    use super::*;
    use crate::mmf::parser;
    use ui::plugin::{EditIntent, EditPlan, EditRequest, EditSelection, EditTransaction};

    fn request(selection: Range<usize>, intent: EditIntent) -> EditRequest {
        EditRequest {
            source_generation: 1,
            cursor_byte: selection.end,
            selection: Some(selection),
            intent,
        }
    }

    fn request_at(cursor_byte: usize, intent: EditIntent) -> EditRequest {
        EditRequest { source_generation: 1, cursor_byte, selection: None, intent }
    }

    fn assert_transaction_inserts(plan: EditPlan, byte: usize, expected_text: &str) {
        let EditPlan::Apply(transaction) = plan else {
            panic!("expected apply transaction");
        };
        assert_eq!(transaction.replacements.len(), 1);
        assert_eq!(transaction.replacements[0].range, byte..byte);
        assert_eq!(transaction.replacements[0].text, expected_text);
    }

    fn assert_deletes_range(plan: EditPlan, expected_range: Range<usize>) {
        let EditPlan::Apply(transaction) = plan else {
            panic!("expected apply transaction");
        };
        assert_eq!(transaction.replacements.len(), 1);
        assert_eq!(transaction.replacements[0].range, expected_range);
        assert!(transaction.replacements[0].text.is_empty());
    }

    fn node_range(tree: &Tree, title: &str) -> Range<usize> {
        collect_nodes_dfs(&tree.root)
            .into_iter()
            .find(|node| node.title == title)
            .expect("fixture must contain node")
            .subtree_source_range
            .clone()
    }

    #[test]
    fn toggle_collapsed_replaces_only_existing_boolean_value() {
        let source = "# Root\n## Child\n```toml node\ncollapsed = false\npriority = \"P1\"\n```\n### Grandchild\n";
        let tree = parser::parse(source).expect("fixture must parse");
        let child = &tree.root.children[0];
        let expected_range = source.find("false").expect("boolean value")
            ..source.find("false").expect("boolean value") + "false".len();
        let EditPlan::Apply(transaction) =
            plan_toggle_collapsed(&tree, source, child.subtree_source_range.clone(), 7)
        else {
            panic!("toggle must apply a transaction");
        };
        assert_eq!(
            transaction.replacements,
            vec![TextReplacement { range: expected_range, text: "true".into() }]
        );
        assert_eq!(transaction.source_generation, 7);
        assert_eq!(transaction.selection_after, EditSelection::Caret(child.title_byte_range.end));
    }

    #[test]
    fn toggle_collapsed_replaces_true_with_false() {
        let source = "# Root\n## Child\n```toml node\ncollapsed = true\n```\n### Grandchild\n";
        let tree = parser::parse(source).expect("fixture must parse");
        let child = &tree.root.children[0];
        let value_start = source.find("true").expect("boolean value");
        let EditPlan::Apply(transaction) =
            plan_toggle_collapsed(&tree, source, child.subtree_source_range.clone(), 8)
        else {
            panic!("toggle must apply a transaction");
        };
        assert_eq!(
            transaction.replacements,
            vec![TextReplacement {
                range: value_start..value_start + "true".len(),
                text: "false".into(),
            }]
        );
    }

    #[test]
    fn toggle_collapsed_inserts_missing_field_into_property_body() {
        let source = "# Root\n## Parent\n```toml node\npriority = \"P1\"\n```\n### Child\n";
        let tree = parser::parse(source).expect("fixture must parse");
        let parent = &tree.root.children[0];
        let insertion_byte = source.find("```\n### Child").expect("closing fence");
        let EditPlan::Apply(transaction) =
            plan_toggle_collapsed(&tree, source, parent.subtree_source_range.clone(), 3)
        else {
            panic!("toggle must apply a transaction");
        };
        assert_eq!(
            transaction.replacements,
            vec![TextReplacement {
                range: insertion_byte..insertion_byte,
                text: "collapsed = true\n".into(),
            }]
        );
    }

    #[test]
    fn toggle_collapsed_inserts_minimal_property_block_after_heading() {
        let source = "# Root\n## Parent\n### Child\n";
        let tree = parser::parse(source).expect("fixture must parse");
        let parent = &tree.root.children[0];
        let insertion_byte = source.find("## Parent").expect("parent heading") + "## Parent".len();
        let EditPlan::Apply(transaction) =
            plan_toggle_collapsed(&tree, source, parent.subtree_source_range.clone(), 4)
        else {
            panic!("toggle must apply a transaction");
        };
        assert_eq!(
            transaction.replacements,
            vec![TextReplacement {
                range: insertion_byte..insertion_byte,
                text: "\n```toml node\ncollapsed = true\n```\n".into(),
            }]
        );
    }

    #[test]
    fn toggle_collapsed_parent_without_props_does_not_use_child_property_block() {
        let source = "# Root\n## Parent\n### Child\n```toml node\ncollapsed = false\n```\n";
        let tree = parser::parse(source).expect("fixture must parse");
        let parent = &tree.root.children[0];
        let insertion_byte = source.find("## Parent").expect("parent heading") + "## Parent".len();
        let EditPlan::Apply(transaction) =
            plan_toggle_collapsed(&tree, source, parent.subtree_source_range.clone(), 5)
        else {
            panic!("toggle must apply a transaction");
        };
        assert_eq!(transaction.replacements[0].range, insertion_byte..insertion_byte);
        assert_eq!(transaction.replacements[0].text, "\n```toml node\ncollapsed = true\n```\n");
    }

    #[test]
    fn toggle_collapsed_preserves_crlf_when_appending_property() {
        let source =
            "# Root\r\n## Parent\r\n```toml node\r\npriority = \"P1\"\r\n```\r\n### Child\r\n";
        let tree = parser::parse(source).expect("fixture must parse");
        let parent = &tree.root.children[0];
        let insertion_byte = source.find("```\r\n### Child").expect("closing fence");
        let EditPlan::Apply(transaction) =
            plan_toggle_collapsed(&tree, source, parent.subtree_source_range.clone(), 6)
        else {
            panic!("toggle must apply a transaction");
        };
        assert_eq!(transaction.replacements[0].range, insertion_byte..insertion_byte);
        assert_eq!(transaction.replacements[0].text, "collapsed = true\r\n");
    }

    #[test]
    fn toggle_collapsed_consumes_root_and_plain_leaf() {
        let source = "# Root\n## Leaf\n";
        let tree = parser::parse(source).expect("fixture must parse");
        assert_eq!(
            plan_toggle_collapsed(&tree, source, tree.root.subtree_source_range.clone(), 1),
            EditPlan::Consume
        );
        let leaf = &tree.root.children[0];
        assert_eq!(
            plan_toggle_collapsed(&tree, source, leaf.subtree_source_range.clone(), 1),
            EditPlan::Consume
        );
    }

    #[test]
    fn toggle_collapsed_consumes_leaf_with_property_block() {
        let source = "# Root\n## Leaf\n```toml node\ncollapsed = false\n```\n";
        let tree = parser::parse(source).expect("fixture must parse");
        let leaf = &tree.root.children[0];
        assert_eq!(
            plan_toggle_collapsed(&tree, source, leaf.subtree_source_range.clone(), 1),
            EditPlan::Consume
        );
    }

    fn apply_transaction_to_text(source: &str, transaction: &EditTransaction) -> String {
        let mut text = source.to_string();
        let mut replacements = transaction.replacements.clone();
        replacements.sort_by_key(|replacement| std::cmp::Reverse(replacement.range.start));
        for replacement in replacements {
            text.replace_range(replacement.range, &replacement.text);
        }
        text
    }

    fn assert_transaction_text_and_selection(
        plan: EditPlan,
        source: &str,
        expected_text: &str,
        expected_selection: &str,
    ) {
        let EditPlan::Apply(transaction) = plan else {
            panic!("expected apply transaction");
        };
        assert_eq!(transaction.replacements.len(), 2);
        assert_eq!(apply_transaction_to_text(source, &transaction), expected_text);
        let EditSelection::Range { anchor, cursor } = transaction.selection_after else {
            panic!("expected range selection");
        };
        assert_eq!(&expected_text[anchor..cursor], expected_selection);
    }

    #[test]
    fn move_subtree_after_sibling_preserves_nested_content_and_selects_it() {
        let source = "# Root\n## A\nA note\n### A1\n## B\n";
        let tree = parser::parse(source).expect("fixture must be valid MMF");
        let plan = plan_move_subtree(
            &tree,
            source,
            node_range(&tree, "A"),
            node_range(&tree, "B"),
            MoveSubtreeTarget::AfterSibling,
            4,
        );
        assert_transaction_text_and_selection(
            plan,
            source,
            "# Root\n## B\n## A\nA note\n### A1\n",
            "## A\nA note\n### A1\n",
        );
    }

    #[test]
    fn move_subtree_as_last_child_increases_every_heading_level() {
        let source = "# Root\n## A\n### A1\n## B\n";
        let tree = parser::parse(source).expect("fixture must be valid MMF");
        let plan = plan_move_subtree(
            &tree,
            source,
            node_range(&tree, "A"),
            node_range(&tree, "B"),
            MoveSubtreeTarget::LastChild,
            9,
        );
        assert_transaction_text_and_selection(
            plan,
            source,
            "# Root\n## B\n### A\n#### A1\n",
            "### A\n#### A1\n",
        );
    }

    #[test]
    fn move_subtree_before_target_child_preserves_requested_child_order() {
        let source = "# Root\n## Source\n## Parent\n### First\n### Last\n";
        let tree = parser::parse(source).expect("fixture must be valid MMF");
        let plan = plan_move_subtree(
            &tree,
            source,
            node_range(&tree, "Source"),
            node_range(&tree, "Last"),
            MoveSubtreeTarget::BeforeChild,
            1,
        );

        assert_transaction_text_and_selection(
            plan,
            source,
            "# Root\n## Parent\n### First\n### Source\n### Last\n",
            "### Source\n",
        );
    }

    #[test]
    fn move_subtree_before_earlier_sibling_sorts_replacements_by_original_coordinates() {
        let source = "# Root\n## A\n## B\n## C\n";
        let tree = parser::parse(source).expect("fixture must be valid MMF");
        let plan = plan_move_subtree(
            &tree,
            source,
            node_range(&tree, "C"),
            node_range(&tree, "A"),
            MoveSubtreeTarget::BeforeSibling,
            1,
        );
        assert_transaction_text_and_selection(plan, source, "# Root\n## C\n## A\n## B\n", "## C\n");
    }

    #[test]
    fn moving_a_sibling_after_its_immediate_predecessor_only_selects_it() {
        let source = "# Root\n## A\n## B\n";
        let tree = parser::parse(source).expect("fixture must be valid MMF");
        let source_range = node_range(&tree, "B");

        assert_eq!(
            plan_move_subtree(
                &tree,
                source,
                source_range.clone(),
                node_range(&tree, "A"),
                MoveSubtreeTarget::AfterSibling,
                1,
            ),
            EditPlan::SetSelection(EditSelection::Range {
                anchor: source_range.start,
                cursor: source_range.end,
            })
        );
    }

    #[test]
    fn moving_next_sibling_as_last_child_reparents_it_when_insertion_matches_source_start() {
        let source = "# Root\n## A\n## B\n";
        let tree = parser::parse(source).expect("fixture must be valid MMF");
        let source_range = node_range(&tree, "B");
        let plan = plan_move_subtree(
            &tree,
            source,
            source_range.clone(),
            node_range(&tree, "A"),
            MoveSubtreeTarget::LastChild,
            1,
        );

        let EditPlan::Apply(transaction) = plan else {
            panic!("moving into the preceding sibling must write a transaction");
        };
        assert_eq!(transaction.replacements.len(), 1);
        assert_eq!(transaction.replacements[0].range, source_range.clone());
        assert_eq!(transaction.replacements[0].text, "### B\n");
        assert_eq!(apply_transaction_to_text(source, &transaction), "# Root\n## A\n### B\n");
        assert_eq!(
            transaction.selection_after,
            EditSelection::Range {
                anchor: source_range.start,
                cursor: source_range.start + "### B\n".len(),
            }
        );
    }

    #[test]
    fn moving_subtree_with_unrepresentable_descendant_level_is_consumed() {
        let source = "x\ny\nz\n";
        let tree = Tree {
            version: 1,
            global_props: Default::default(),
            root: Node {
                title: "Root".into(),
                children: vec![
                    Node {
                        title: "Source".into(),
                        children: vec![Node {
                            title: "Descendant".into(),
                            children: vec![],
                            props: None,
                            note: None,
                            source_range: 2..4,
                            subtree_source_range: 2..4,
                            title_byte_range: 3..3,
                            heading_marker_range: 2..3,
                            child_insertion_byte: 4,
                            heading_level: u8::MAX,
                            property_source: None,
                            heading_source_end: 0,
                        }],
                        props: None,
                        note: None,
                        source_range: 0..4,
                        subtree_source_range: 0..4,
                        title_byte_range: 1..1,
                        heading_marker_range: 0..1,
                        child_insertion_byte: 2,
                        heading_level: u8::MAX - 1,
                        property_source: None,
                        heading_source_end: 0,
                    },
                    Node {
                        title: "Anchor".into(),
                        children: vec![],
                        props: None,
                        note: None,
                        source_range: 4..6,
                        subtree_source_range: 4..6,
                        title_byte_range: 5..5,
                        heading_marker_range: 4..5,
                        child_insertion_byte: 6,
                        heading_level: u8::MAX - 1,
                        property_source: None,
                        heading_source_end: 0,
                    },
                ],
                props: None,
                note: None,
                source_range: 0..6,
                subtree_source_range: 0..6,
                title_byte_range: 0..0,
                heading_marker_range: 0..0,
                child_insertion_byte: 0,
                heading_level: 1,
                property_source: None,
                heading_source_end: 0,
            },
        };
        let source_range = tree.root.children[0].subtree_source_range.clone();
        let anchor_range = tree.root.children[1].subtree_source_range.clone();

        assert_eq!(
            plan_move_subtree(
                &tree,
                source,
                source_range,
                anchor_range,
                MoveSubtreeTarget::LastChild,
                1,
            ),
            EditPlan::Consume
        );
    }

    #[test]
    fn moving_unterminated_subtree_to_document_end_excludes_inserted_separator_from_selection() {
        let source = "# Root\n## A\n## B";
        let tree = parser::parse(source).expect("fixture must be valid MMF");
        let plan = plan_move_subtree(
            &tree,
            source,
            node_range(&tree, "B"),
            tree.root.subtree_source_range.clone(),
            MoveSubtreeTarget::LastChild,
            1,
        );

        assert_transaction_text_and_selection(plan, source, "# Root\n## A\n\n## B", "## B");
    }

    #[test]
    fn move_subtree_before_sibling_with_a_different_parent_promotes_the_source() {
        let source = "# Root\n## A\n### A1\n## B\n### B1\n";
        let tree = parser::parse(source).expect("fixture must be valid MMF");

        let plan = plan_move_subtree(
            &tree,
            source,
            node_range(&tree, "A1"),
            node_range(&tree, "B"),
            MoveSubtreeTarget::BeforeSibling,
            1,
        );
        assert_transaction_text_and_selection(
            plan,
            source,
            "# Root\n## A\n## A1\n## B\n### B1\n",
            "## A1\n",
        );
    }

    #[test]
    fn move_subtree_rejects_the_root() {
        let source = "# Root\n## A\n";
        let tree = parser::parse(source).expect("fixture must be valid MMF");

        assert_eq!(
            plan_move_subtree(
                &tree,
                source,
                tree.root.subtree_source_range.clone(),
                node_range(&tree, "A"),
                MoveSubtreeTarget::AfterSibling,
                1,
            ),
            EditPlan::Consume
        );
    }

    #[test]
    fn move_subtree_rejects_the_root_as_a_sibling_anchor() {
        let source = "# Root\n## A\n";
        let tree = parser::parse(source).expect("fixture must be valid MMF");

        assert_eq!(
            plan_move_subtree(
                &tree,
                source,
                node_range(&tree, "A"),
                tree.root.subtree_source_range.clone(),
                MoveSubtreeTarget::BeforeSibling,
                1,
            ),
            EditPlan::Consume
        );
    }

    #[test]
    fn move_subtree_rejects_a_descendant_as_anchor() {
        let source = "# Root\n## A\n### A1\n## B\n";
        let tree = parser::parse(source).expect("fixture must be valid MMF");

        assert_eq!(
            plan_move_subtree(
                &tree,
                source,
                node_range(&tree, "A"),
                node_range(&tree, "A1"),
                MoveSubtreeTarget::LastChild,
                1,
            ),
            EditPlan::Consume
        );
    }

    #[test]
    fn move_subtree_preserves_crlf_boundaries() {
        let source = "# Root\r\n## A\r\n## B\r\n";
        let tree = parser::parse(source).expect("fixture must be valid MMF");
        let plan = plan_move_subtree(
            &tree,
            source,
            node_range(&tree, "A"),
            node_range(&tree, "B"),
            MoveSubtreeTarget::AfterSibling,
            1,
        );
        assert_transaction_text_and_selection(
            plan,
            source,
            "# Root\r\n## B\r\n## A\r\n",
            "## A\r\n",
        );
    }

    #[test]
    fn move_subtree_preserves_properties_and_code_fence_notes() {
        let source = concat!(
            "# Root\n",
            "## A\n",
            "```toml node\n",
            "id = \"a\"\n",
            "```\n",
            "```rust\n",
            "# not a heading\n",
            "```\n",
            "### A1\n",
            "## B\n",
        );
        let tree = parser::parse(source).expect("fixture must be valid MMF");
        let plan = plan_move_subtree(
            &tree,
            source,
            node_range(&tree, "A"),
            node_range(&tree, "B"),
            MoveSubtreeTarget::AfterSibling,
            1,
        );
        assert_transaction_text_and_selection(
            plan,
            source,
            concat!(
                "# Root\n",
                "## B\n",
                "## A\n",
                "```toml node\n",
                "id = \"a\"\n",
                "```\n",
                "```rust\n",
                "# not a heading\n",
                "```\n",
                "### A1\n",
            ),
            concat!(
                "## A\n",
                "```toml node\n",
                "id = \"a\"\n",
                "```\n",
                "```rust\n",
                "# not a heading\n",
                "```\n",
                "### A1\n",
            ),
        );
    }

    #[test]
    fn selected_node_typing_replaces_only_title() {
        let source = "# Root\n## Parent\n### Child\n";
        let tree = parser::parse(source).expect("parse");
        let parent = &tree.root.children[0];
        let request =
            request(parent.subtree_source_range.clone(), EditIntent::InsertText("Renamed".into()));

        assert_eq!(
            plan_mindmap_edit(&tree, source, &request),
            EditPlan::Apply(EditTransaction::replace(
                request.source_generation,
                parent.title_byte_range.clone(),
                "Renamed".into(),
                parent.title_byte_range.start + "Renamed".len(),
            ))
        );
    }

    #[test]
    fn title_text_selection_typing_replaces_only_selected_text() {
        let source = "# Root\n## Parent\n";
        let tree = parser::parse(source).expect("parse");
        let parent = &tree.root.children[0];
        let selection = parent.title_byte_range.start + 1..parent.title_byte_range.start + 4;
        let request = request(selection.clone(), EditIntent::InsertText("X".into()));

        assert_eq!(
            plan_mindmap_edit(&tree, source, &request),
            EditPlan::Apply(EditTransaction::replace(
                request.source_generation,
                selection,
                "X".into(),
                parent.title_byte_range.start + 2,
            ))
        );
    }

    #[test]
    fn title_text_selection_delete_removes_only_selected_text() {
        let source = "# Root\n## Parent\n";
        let tree = parser::parse(source).expect("parse");
        let parent = &tree.root.children[0];
        let selection = parent.title_byte_range.start + 1..parent.title_byte_range.start + 4;
        let request = request(selection.clone(), EditIntent::DeleteBackward);

        assert_eq!(
            plan_mindmap_edit(&tree, source, &request),
            EditPlan::Apply(EditTransaction::replace(
                request.source_generation,
                selection.clone(),
                String::new(),
                selection.start,
            ))
        );
    }

    #[test]
    fn tab_inserts_empty_child_at_child_insertion_byte() {
        let source = "# Root\n## Parent\nparent note\n### Existing\n";
        let tree = parser::parse(source).expect("parse");
        let parent = &tree.root.children[0];
        let request = request_at(parent.title_byte_range.end, EditIntent::Indent);

        assert_transaction_inserts(
            plan_mindmap_edit(&tree, source, &request),
            parent.child_insertion_byte,
            "###\n",
        );
    }

    #[test]
    fn leaf_body_new_child_inserts_at_subtree_end_and_places_caret_in_empty_title() {
        let source = "# Root\n## Leaf\nleaf body\n";
        let tree = parser::parse(source).expect("parse");
        let leaf = &tree.root.children[0];
        let request = request_at(leaf.title_byte_range.end, EditIntent::Indent);

        assert_eq!(leaf.child_insertion_byte, leaf.subtree_source_range.end);
        let EditPlan::Apply(transaction) = plan_mindmap_edit(&tree, source, &request) else {
            panic!("indent should return a transaction");
        };
        assert_eq!(transaction.replacements.len(), 1);
        assert_eq!(
            transaction.replacements[0].range,
            leaf.child_insertion_byte..leaf.child_insertion_byte
        );
        assert_eq!(transaction.replacements[0].text, "###\n");
        assert_eq!(
            transaction.selection_after,
            EditSelection::Caret(leaf.child_insertion_byte + "###".len())
        );
    }

    #[test]
    fn demote_changes_current_subtree_markers_as_one_transaction() {
        let source = "# Root\n## First\n## Second\n### Leaf\n";
        let tree = parser::parse(source).expect("parse");
        let second = &tree.root.children[1];
        let request = request_at(second.title_byte_range.start, EditIntent::DemoteObject);

        let EditPlan::Apply(transaction) = plan_mindmap_edit(&tree, source, &request) else {
            panic!("demote should return a transaction");
        };
        assert_eq!(transaction.replacements.len(), 2);
        assert!(transaction.replacements.iter().all(|replacement| replacement.text == "#"));
        assert_eq!(
            transaction
                .replacements
                .iter()
                .map(|replacement| replacement.range.clone())
                .collect::<Vec<_>>(),
            vec![
                second.heading_marker_range.end..second.heading_marker_range.end,
                second.children[0].heading_marker_range.end
                    ..second.children[0].heading_marker_range.end,
            ]
        );
        assert_eq!(
            transaction.selection_after,
            EditSelection::Caret(second.title_byte_range.start + 1)
        );
    }

    #[test]
    fn promotion_updates_selected_subtree_range_after_all_marker_removals() {
        let source = "# Root\n## Parent\n### Leaf\n";
        let tree = parser::parse(source).expect("parse");
        let parent = &tree.root.children[0];
        let request = request(parent.subtree_source_range.clone(), EditIntent::PromoteObject);

        let EditPlan::Apply(transaction) = plan_mindmap_edit(&tree, source, &request) else {
            panic!("promotion should return a transaction");
        };
        assert_eq!(transaction.replacements.len(), 2);
        assert_eq!(
            transaction
                .replacements
                .iter()
                .map(|replacement| replacement.range.clone())
                .collect::<Vec<_>>(),
            vec![
                parent.heading_marker_range.end - 1..parent.heading_marker_range.end,
                parent.children[0].heading_marker_range.end - 1
                    ..parent.children[0].heading_marker_range.end,
            ]
        );
        assert_eq!(
            transaction.selection_after,
            EditSelection::Range {
                anchor: parent.subtree_source_range.start,
                cursor: parent.subtree_source_range.end - 2,
            }
        );
    }

    #[test]
    fn selected_parent_delete_removes_whole_subtree() {
        let source = "# Root\n## Parent\n### Child\n## Next\n";
        let tree = parser::parse(source).expect("parse");
        let parent = &tree.root.children[0];
        let request = request(parent.subtree_source_range.clone(), EditIntent::DeleteForward);

        assert_deletes_range(
            plan_mindmap_edit(&tree, source, &request),
            parent.subtree_source_range.clone(),
        );
    }

    #[test]
    fn deleting_a_selected_subtree_selects_the_previous_visible_dfs_node() {
        let source =
            "# Root\n## Previous\n### Previous child\n## Deleted\n### Descendant\n## Next\n";
        let tree = parser::parse(source).expect("parse");
        let deleted = &tree.root.children[1];
        let previous = &tree.root.children[0].children[0];
        let request = request(deleted.subtree_source_range.clone(), EditIntent::DeleteBackward);

        let EditPlan::Apply(transaction) = plan_mindmap_edit(&tree, source, &request) else {
            panic!("selected subtree deletion must be transactional");
        };
        assert_eq!(
            transaction.selection_after,
            EditSelection::Range {
                anchor: previous.subtree_source_range.start,
                cursor: previous.subtree_source_range.end,
            }
        );
    }

    #[test]
    fn deleting_the_first_child_selects_its_parent_object() {
        let source = "# Root\n## Deleted\n### Descendant\n## Next\n";
        let tree = parser::parse(source).expect("parse");
        let deleted = &tree.root.children[0];
        let request = request(deleted.subtree_source_range.clone(), EditIntent::DeleteForward);

        let EditPlan::Apply(transaction) = plan_mindmap_edit(&tree, source, &request) else {
            panic!("selected subtree deletion must be transactional");
        };
        assert_eq!(
            transaction.selection_after,
            EditSelection::Range {
                anchor: tree.root.subtree_source_range.start,
                cursor: tree.root.subtree_source_range.end - deleted.subtree_source_range.len(),
            }
        );
    }

    #[test]
    fn root_cannot_be_deleted_or_releveled() {
        let source = "# Root\n## Child\n";
        let tree = parser::parse(source).expect("parse");
        let selection = tree.root.subtree_source_range.clone();

        for intent in [
            EditIntent::DeleteBackward,
            EditIntent::DeleteForward,
            EditIntent::PromoteObject,
            EditIntent::DemoteObject,
        ] {
            assert_eq!(
                plan_mindmap_edit(&tree, source, &request(selection.clone(), intent)),
                EditPlan::Consume
            );
        }
    }

    #[test]
    fn first_sibling_cannot_be_demoted() {
        let source = "# Root\n## First\n## Second\n";
        let tree = parser::parse(source).expect("parse");
        let first = &tree.root.children[0];

        assert_eq!(
            plan_mindmap_edit(
                &tree,
                source,
                &request_at(first.title_byte_range.start, EditIntent::DemoteObject),
            ),
            EditPlan::Consume
        );
    }

    #[test]
    fn title_editing_deletion_stays_within_title() {
        let source = "# Root\n## Parent\n";
        let tree = parser::parse(source).expect("parse");
        let parent = &tree.root.children[0];

        assert_eq!(
            plan_mindmap_edit(
                &tree,
                source,
                &request_at(parent.title_byte_range.start, EditIntent::DeleteBackward),
            ),
            EditPlan::Consume
        );
        assert_eq!(
            plan_mindmap_edit(
                &tree,
                source,
                &request_at(parent.title_byte_range.end, EditIntent::DeleteForward),
            ),
            EditPlan::Consume
        );
    }

    #[test]
    fn title_backspace_deletes_one_extended_grapheme_cluster() {
        let source = "# Root\n## Cafe\u{301}\n";
        let tree = parser::parse(source).expect("parse");
        let child = &tree.root.children[0];
        let request = request_at(child.title_byte_range.end, EditIntent::DeleteBackward);

        assert_eq!(
            plan_mindmap_edit(&tree, source, &request),
            EditPlan::Apply(EditTransaction::replace(
                request.source_generation,
                child.title_byte_range.start + "Caf".len()..child.title_byte_range.end,
                String::new(),
                child.title_byte_range.start + "Caf".len(),
            ))
        );
    }

    #[test]
    fn title_delete_deletes_one_zwj_grapheme_cluster() {
        let source = "# Root\n## \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}x\n";
        let tree = parser::parse(source).expect("parse");
        let child = &tree.root.children[0];
        let request = request_at(child.title_byte_range.start, EditIntent::DeleteForward);

        assert_eq!(
            plan_mindmap_edit(&tree, source, &request),
            EditPlan::Apply(EditTransaction::replace(
                request.source_generation,
                child.title_byte_range.start
                    ..child.title_byte_range.start
                        + "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}".len(),
                String::new(),
                child.title_byte_range.start,
            ))
        );
    }

    #[test]
    fn enter_inserts_empty_sibling_with_document_line_ending() {
        let source = "# Root\r\n## Parent\r\n";
        let tree = parser::parse(source).expect("parse");
        let parent = &tree.root.children[0];
        let request = request_at(parent.title_byte_range.end, EditIntent::InsertParagraphBreak);

        assert_transaction_inserts(
            plan_mindmap_edit(&tree, source, &request),
            parent.subtree_source_range.end,
            "##\r\n",
        );
    }

    #[test]
    fn selected_node_enter_moves_caret_to_title_end_without_writing() {
        let source = "# Root\n## Parent\n";
        let tree = parser::parse(source).expect("parse");
        let parent = &tree.root.children[0];

        assert_eq!(
            plan_mindmap_edit(
                &tree,
                source,
                &request(parent.subtree_source_range.clone(), EditIntent::InsertParagraphBreak),
            ),
            EditPlan::SetSelection(EditSelection::Caret(parent.title_byte_range.end))
        );
    }

    #[test]
    fn title_editing_select_object_selects_current_subtree_without_writing() {
        let source = "# Root\n## Parent\n### Child\n";
        let tree = parser::parse(source).expect("parse");
        let parent = &tree.root.children[0];

        assert_eq!(
            plan_mindmap_edit(
                &tree,
                source,
                &request_at(parent.title_byte_range.end, EditIntent::SelectObject),
            ),
            EditPlan::SetSelection(EditSelection::Range {
                anchor: parent.subtree_source_range.start,
                cursor: parent.subtree_source_range.end,
            })
        );
    }

    fn applied_source(source: &str, plan: EditPlan) -> (String, EditSelection) {
        let EditPlan::Apply(transaction) = plan else {
            panic!("expected an applied theme transaction");
        };
        let mut result = source.to_owned();
        let mut replacements = transaction.replacements;
        replacements.sort_by_key(|replacement| replacement.range.start);
        for replacement in replacements.into_iter().rev() {
            result.replace_range(replacement.range, &replacement.text);
        }
        (result, transaction.selection_after)
    }

    #[test]
    fn set_theme_replaces_only_existing_theme_literal() {
        let source = "```toml mindmap\nversion = 1\ntheme = \"dawn\"\n```\n\n# Root\n";
        let tree = parser::parse(source).expect("fixture must parse");
        let (result, _) =
            applied_source(source, plan_set_mindmap_theme(&tree, source, "tide", 7, source.len()));
        assert_eq!(result, "```toml mindmap\nversion = 1\ntheme = \"tide\"\n```\n\n# Root\n");
    }

    #[test]
    fn set_theme_replaces_existing_single_quoted_theme_literal() {
        let source = "```toml mindmap\nversion = 1\ntheme = 'dawn'\n```\n\n# Root\n";
        let tree = parser::parse(source).expect("fixture must parse");
        let (result, _) =
            applied_source(source, plan_set_mindmap_theme(&tree, source, "tide", 7, source.len()));

        assert_eq!(result.matches("theme =").count(), 1);
        assert_eq!(result, "```toml mindmap\nversion = 1\ntheme = \"tide\"\n```\n\n# Root\n");
        assert_eq!(
            parser::parse(&result)
                .expect("updated fixture must parse")
                .global_props
                .get("theme")
                .map(String::as_str),
            Some("tide")
        );
    }

    #[test]
    fn set_theme_replaces_existing_single_line_triple_quoted_theme_literal() {
        let source = "```toml mindmap\nversion = 1\ntheme = \"\"\"dawn\"\"\"\n```\n\n# Root\n";
        let tree = parser::parse(source).expect("fixture must parse");
        let (result, _) =
            applied_source(source, plan_set_mindmap_theme(&tree, source, "tide", 7, source.len()));

        assert_eq!(result.matches("theme =").count(), 1);
        assert_eq!(result, "```toml mindmap\nversion = 1\ntheme = \"tide\"\n```\n\n# Root\n");
        assert_eq!(
            parser::parse(&result)
                .expect("updated fixture must parse")
                .global_props
                .get("theme")
                .map(String::as_str),
            Some("tide")
        );
    }

    #[test]
    fn set_theme_inserts_field_before_existing_closing_fence() {
        let source = "```toml mindmap\nversion = 1\nlayout = \"auto\"\n```\n\n# Root\n";
        let tree = parser::parse(source).expect("fixture must parse");
        let (result, _) =
            applied_source(source, plan_set_mindmap_theme(&tree, source, "amber", 3, source.len()));
        assert_eq!(
            result,
            "```toml mindmap\nversion = 1\nlayout = \"auto\"\ntheme = \"amber\"\n```\n\n# Root\n"
        );
    }

    #[test]
    fn set_theme_creates_global_block_before_root() {
        let source = "# Root\n";
        let tree = parser::parse(source).expect("fixture must parse");
        let (result, _) =
            applied_source(source, plan_set_mindmap_theme(&tree, source, "tide", 1, source.len()));
        assert_eq!(result, "```toml mindmap\nversion = 1\ntheme = \"tide\"\n```\n\n# Root\n");
    }

    #[test]
    fn set_theme_preserves_crlf_newlines() {
        let source = "```toml mindmap\r\nversion = 1\r\n```\r\n# Root\r\n";
        let tree = parser::parse(source).expect("fixture must parse");
        let (result, _) =
            applied_source(source, plan_set_mindmap_theme(&tree, source, "iris", 1, source.len()));
        assert_eq!(
            result,
            "```toml mindmap\r\nversion = 1\r\ntheme = \"iris\"\r\n```\r\n# Root\r\n"
        );
    }

    #[test]
    fn selecting_current_theme_returns_consume() {
        let source = "```toml mindmap\ntheme = \"dawn\"\n```\n\n# Root\n";
        let tree = parser::parse(source).expect("fixture must parse");
        assert_eq!(
            plan_set_mindmap_theme(&tree, source, "dawn", 1, source.len()),
            EditPlan::Consume
        );
    }

    #[test]
    fn set_theme_adjusts_caret_when_insertion_precedes_cursor() {
        let source = "# Root\n";
        let tree = parser::parse(source).expect("fixture must parse");
        let original_cursor = source.find("Root").expect("root title") + 2;
        let (result, selection) = applied_source(
            source,
            plan_set_mindmap_theme(&tree, source, "tide", 1, original_cursor),
        );
        let EditSelection::Caret(cursor_after) = selection else {
            panic!("theme edit must preserve a caret");
        };
        assert_eq!(&result[cursor_after - 2..cursor_after + 2], "Root");
    }
}
