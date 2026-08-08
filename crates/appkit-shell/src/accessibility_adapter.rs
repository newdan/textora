//! 将 `textora-ui` 的平台无关语义树桥接到 AccessKit。

use std::collections::HashMap;

use accesskit::{
    Action, ActionData, ActionRequest, Node, NodeId, Orientation, Role, Toggled, Tree, TreeId,
    TreeUpdate,
};
use ui::core::widget::{SensitiveText, TextPayload};
use ui::core::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityId, AccessibilityNode,
    AccessibilityOrientation, AccessibilityRole, AccessibilityTree,
};

#[derive(Clone)]
struct ActionTarget {
    actions: Vec<AccessibilityAction>,
    sensitive: bool,
}

pub struct AccessibilitySnapshot {
    pub update: TreeUpdate,
    actions: HashMap<AccessibilityId, ActionTarget>,
}

impl AccessibilitySnapshot {
    pub fn translate_action(&self, request: &ActionRequest) -> Option<AccessibilityActionRequest> {
        translate_action(request, &self.actions)
    }
}

fn translate_action(
    request: &ActionRequest,
    actions: &HashMap<AccessibilityId, ActionTarget>,
) -> Option<AccessibilityActionRequest> {
    if request.target_tree != TreeId::ROOT {
        return None;
    }
    let target = AccessibilityId(u64::from(request.target_node));
    let capabilities = actions.get(&target)?;
    let action = match request.action {
        Action::Focus => supported_action(capabilities, AccessibilityAction::Focus)?,
        Action::Click => supported_action(capabilities, AccessibilityAction::Toggle)
            .or_else(|| supported_action(capabilities, AccessibilityAction::Activate))?,
        Action::Increment => supported_action(capabilities, AccessibilityAction::Increment)?,
        Action::Decrement => supported_action(capabilities, AccessibilityAction::Decrement)?,
        Action::SetValue => supported_action(capabilities, AccessibilityAction::SetValue)?,
        _ => return None,
    };
    if action != AccessibilityAction::SetValue {
        return Some(AccessibilityActionRequest::new(target, action));
    }

    let ActionData::Value(value) = request.data.as_ref()? else {
        return None;
    };
    let value = if capabilities.sensitive {
        TextPayload::Sensitive(SensitiveText::new(value.to_string()))
    } else {
        TextPayload::Plain(value.to_string())
    };
    Some(AccessibilityActionRequest { target, action, value: Some(value) })
}

pub fn build_accessibility_snapshot(tree: &AccessibilityTree) -> AccessibilitySnapshot {
    let mut nodes = Vec::new();
    let mut actions = HashMap::new();
    collect_platform_nodes(&tree.root, &mut nodes, &mut actions);
    let root_id = NodeId(tree.root.id.0);
    let focus = tree.focus.map_or(root_id, |id| NodeId(id.0));
    let mut platform_tree = Tree::new(root_id);
    platform_tree.toolkit_name = Some("textora".into());

    AccessibilitySnapshot {
        update: TreeUpdate { nodes, tree: Some(platform_tree), tree_id: TreeId::ROOT, focus },
        actions,
    }
}

fn collect_platform_nodes(
    node: &AccessibilityNode,
    output: &mut Vec<(NodeId, Node)>,
    actions: &mut HashMap<AccessibilityId, ActionTarget>,
) {
    let mut platform_node = Node::new(platform_role(node));
    platform_node.set_bounds(accesskit::Rect::new(
        f64::from(node.bounds.x),
        f64::from(node.bounds.y),
        f64::from(node.bounds.right()),
        f64::from(node.bounds.bottom()),
    ));
    if let Some(name) = &node.name {
        if node.role == AccessibilityRole::StaticText {
            platform_node.set_value(name.clone());
        } else {
            platform_node.set_label(name.clone());
        }
    }
    if let Some(description) = &node.description {
        platform_node.set_description(description.clone());
    }
    if !node.state.sensitive
        && let Some(value) = &node.value
    {
        platform_node.set_value(value.clone());
    }
    if let Some(value) = node.numeric_value {
        platform_node.set_numeric_value(value);
    }
    if let Some(minimum) = node.numeric_minimum {
        platform_node.set_min_numeric_value(minimum);
    }
    if let Some(maximum) = node.numeric_maximum {
        platform_node.set_max_numeric_value(maximum);
    }
    if let Some(orientation) = node.orientation {
        platform_node.set_orientation(match orientation {
            AccessibilityOrientation::Horizontal => Orientation::Horizontal,
            AccessibilityOrientation::Vertical => Orientation::Vertical,
        });
    }
    if node.state.disabled {
        platform_node.set_disabled();
    }
    if node.state.read_only {
        platform_node.set_read_only();
    }
    if let Some(checked) = node.state.checked {
        platform_node.set_toggled(if checked { Toggled::True } else { Toggled::False });
    }
    if let Some(selected) = node.state.selected {
        platform_node.set_selected(selected);
    }
    if let Some(expanded) = node.state.expanded {
        platform_node.set_expanded(expanded);
    }
    if node.role == AccessibilityRole::Dialog {
        platform_node.set_modal();
    }
    platform_node
        .set_labelled_by(node.labelled_by.iter().map(|id| NodeId(id.0)).collect::<Vec<_>>());
    platform_node
        .set_described_by(node.described_by.iter().map(|id| NodeId(id.0)).collect::<Vec<_>>());
    platform_node
        .set_children(node.children.iter().map(|child| NodeId(child.id.0)).collect::<Vec<_>>());
    for action in &node.actions {
        platform_node.add_action(platform_action(*action));
    }
    actions.insert(
        node.id,
        ActionTarget { actions: node.actions.clone(), sensitive: node.state.sensitive },
    );
    output.push((NodeId(node.id.0), platform_node));
    for child in &node.children {
        collect_platform_nodes(child, output, actions);
    }
}

fn platform_role(node: &AccessibilityNode) -> Role {
    match node.role {
        AccessibilityRole::Window => Role::Window,
        AccessibilityRole::Group => Role::Group,
        AccessibilityRole::Button => Role::Button,
        AccessibilityRole::CheckBox => Role::CheckBox,
        AccessibilityRole::Switch => Role::Switch,
        AccessibilityRole::TextField if node.state.sensitive => Role::PasswordInput,
        AccessibilityRole::TextField => Role::TextInput,
        AccessibilityRole::StaticText => Role::Label,
        AccessibilityRole::Tooltip => Role::Tooltip,
        AccessibilityRole::Slider => Role::Slider,
        AccessibilityRole::ScrollBar => Role::ScrollBar,
        AccessibilityRole::List => Role::List,
        AccessibilityRole::ListItem => Role::ListItem,
        AccessibilityRole::Tree => Role::Tree,
        AccessibilityRole::TreeItem => Role::TreeItem,
        AccessibilityRole::Menu => Role::Menu,
        AccessibilityRole::MenuItem => Role::MenuItem,
        AccessibilityRole::Dialog => Role::Dialog,
        AccessibilityRole::Toolbar => Role::Toolbar,
        AccessibilityRole::Separator => Role::Splitter,
    }
}

fn platform_action(action: AccessibilityAction) -> Action {
    match action {
        AccessibilityAction::Focus => Action::Focus,
        AccessibilityAction::Activate | AccessibilityAction::Toggle => Action::Click,
        AccessibilityAction::Increment => Action::Increment,
        AccessibilityAction::Decrement => Action::Decrement,
        AccessibilityAction::SetValue => Action::SetValue,
    }
}

fn supported_action(
    target: &ActionTarget,
    action: AccessibilityAction,
) -> Option<AccessibilityAction> {
    target.actions.contains(&action).then_some(action)
}

pub struct PlatformAccessibilityAdapter {
    adapter: accesskit_winit::Adapter,
    actions: HashMap<AccessibilityId, ActionTarget>,
}

impl PlatformAccessibilityAdapter {
    pub fn new<T>(
        event_loop: &winit::event_loop::ActiveEventLoop,
        window: &winit::window::Window,
        proxy: winit::event_loop::EventLoopProxy<T>,
    ) -> Self
    where
        T: From<accesskit_winit::Event> + Send + 'static,
    {
        Self {
            adapter: accesskit_winit::Adapter::with_event_loop_proxy(event_loop, window, proxy),
            actions: HashMap::new(),
        }
    }

    pub fn process_window_event(
        &mut self,
        window: &winit::window::Window,
        event: &winit::event::WindowEvent,
    ) {
        self.adapter.process_event(window, event);
    }

    pub fn update(&mut self, tree: &AccessibilityTree) {
        let snapshot = build_accessibility_snapshot(tree);
        self.actions = snapshot.actions;
        self.adapter.update_if_active(move || snapshot.update);
    }

    pub fn translate_action(&self, request: &ActionRequest) -> Option<AccessibilityActionRequest> {
        translate_action(request, &self.actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use accesskit::{Action, ActionData, Role, Toggled, TreeId};
    use ui::core::widget::TextPayload;
    use ui::core::{AccessibilityNode, AccessibilityRole, AccessibilityTree, Rect};

    #[test]
    fn snapshot_preserves_tree_structure_state_relations_bounds_and_focus() {
        let label_id = AccessibilityId(2);
        let checkbox_id = AccessibilityId(3);
        let root = AccessibilityNode::new(
            AccessibilityId(1),
            AccessibilityRole::Window,
            Rect::new(0.0, 0.0, 640.0, 480.0),
        )
        .with_child(
            AccessibilityNode::new(
                label_id,
                AccessibilityRole::StaticText,
                Rect::new(10.0, 10.0, 120.0, 24.0),
            )
            .with_name("自动保存"),
        )
        .with_child(
            AccessibilityNode::new(
                checkbox_id,
                AccessibilityRole::CheckBox,
                Rect::new(150.0, 10.0, 20.0, 20.0),
            )
            .with_disabled(false)
            .with_focused(true)
            .with_checked(true)
            .with_labelled_by(label_id)
            .with_action(AccessibilityAction::Focus)
            .with_action(AccessibilityAction::Toggle),
        );
        let snapshot =
            build_accessibility_snapshot(&AccessibilityTree::new(root, Some(checkbox_id)));

        assert_eq!(snapshot.update.tree.as_ref().unwrap().root, NodeId(1));
        assert_eq!(snapshot.update.focus, NodeId(3));
        assert_eq!(snapshot.update.nodes.len(), 3);
        let root = platform_node(&snapshot, 1);
        assert_eq!(root.role(), Role::Window);
        assert_eq!(root.children(), &[NodeId(2), NodeId(3)]);
        let checkbox = platform_node(&snapshot, 3);
        assert_eq!(checkbox.role(), Role::CheckBox);
        assert_eq!(checkbox.toggled(), Some(Toggled::True));
        assert_eq!(checkbox.labelled_by(), &[NodeId(2)]);
        assert!(checkbox.supports_action(Action::Focus));
        assert!(checkbox.supports_action(Action::Click));
        assert_eq!(checkbox.bounds().unwrap().x0, 150.0);
    }

    #[test]
    fn platform_actions_resolve_to_the_original_shared_action_contract() {
        let toggle_id = AccessibilityId(8);
        let text_id = AccessibilityId(9);
        let root = AccessibilityNode::new(
            AccessibilityId(1),
            AccessibilityRole::Window,
            Rect::new(0.0, 0.0, 640.0, 480.0),
        )
        .with_child(
            AccessibilityNode::new(
                toggle_id,
                AccessibilityRole::Switch,
                Rect::new(10.0, 10.0, 40.0, 20.0),
            )
            .with_action(AccessibilityAction::Toggle),
        )
        .with_child(
            AccessibilityNode::new(
                text_id,
                AccessibilityRole::TextField,
                Rect::new(10.0, 40.0, 200.0, 28.0),
            )
            .with_action(AccessibilityAction::SetValue),
        );
        let snapshot = build_accessibility_snapshot(&AccessibilityTree::new(root, None));

        assert_eq!(
            snapshot.translate_action(&ActionRequest {
                action: Action::Click,
                target_tree: TreeId::ROOT,
                target_node: NodeId(toggle_id.0),
                data: None,
            }),
            Some(AccessibilityActionRequest::new(toggle_id, AccessibilityAction::Toggle))
        );
        assert_eq!(
            snapshot.translate_action(&ActionRequest {
                action: Action::SetValue,
                target_tree: TreeId::ROOT,
                target_node: NodeId(text_id.0),
                data: Some(ActionData::Value("新值".into())),
            }),
            Some(AccessibilityActionRequest::set_value(
                text_id,
                TextPayload::Plain("新值".to_owned()),
            ))
        );
    }

    fn platform_node(snapshot: &AccessibilitySnapshot, id: u64) -> &accesskit::Node {
        &snapshot
            .update
            .nodes
            .iter()
            .find(|(node_id, _)| *node_id == NodeId(id))
            .expect("snapshot should contain requested node")
            .1
    }
}
