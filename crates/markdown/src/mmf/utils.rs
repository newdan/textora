use super::model::{Node, Tree};

/// Returns all nodes in DFS pre-order (root first).
pub(crate) fn collect_nodes_dfs(node: &Node) -> Vec<&Node> {
    let mut out = vec![node];
    for child in &node.children {
        out.extend(collect_nodes_dfs(child));
    }
    out
}

/// Returns titles for every node in DFS pre-order.
pub(crate) fn collect_dfs_titles(node: &Node) -> Vec<String> {
    let mut out = vec![node.title.clone()];
    for child in &node.children {
        out.extend(collect_dfs_titles(child));
    }
    out
}

/// Finds the parent node for the node at `target_idx` in DFS order.
pub(crate) fn find_parent(tree: &Tree, target_idx: usize) -> Option<&Node> {
    let nodes = collect_nodes_dfs(&tree.root);
    let target = nodes.get(target_idx)?;
    find_parent_of(&tree.root, target)
}

/// Recursive helper: returns the parent that directly owns `target`.
pub(crate) fn find_parent_of<'a>(node: &'a Node, target: &Node) -> Option<&'a Node> {
    for child in &node.children {
        if std::ptr::eq(child as *const Node, target as *const Node) {
            return Some(node);
        }
        if let Some(parent) = find_parent_of(child, target) {
            return Some(parent);
        }
    }
    None
}

/// Returns sibling DFS indices for the node at `target_idx` (including itself).
pub(crate) fn find_siblings(tree: &Tree, target_idx: usize) -> Option<Vec<usize>> {
    let parent = find_parent(tree, target_idx)?;
    let nodes = collect_nodes_dfs(&tree.root);
    let mut siblings = Vec::new();
    for child in &parent.children {
        if let Some(idx) = nodes.iter().position(|n| std::ptr::eq(*n, child)) {
            siblings.push(idx);
        }
    }
    Some(siblings)
}
