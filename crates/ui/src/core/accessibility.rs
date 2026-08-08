//! 平台无关的可访问性语义模型。
//!
//! 所有矩形均使用屏幕物理像素；平台适配器只能读取这里生成并验证过的树。

use std::collections::HashSet;

use crate::core::geom::Rect;
use crate::core::widget::{TextPayload, WidgetId};

/// 可访问性树中的稳定节点标识。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AccessibilityId(pub u64);

impl AccessibilityId {
    /// 为一个 widget 的稳定子节点生成确定性标识。
    pub fn child(self, discriminator: u64) -> Self {
        let mut mixed = self.0 ^ discriminator.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        Self(mixed ^ (mixed >> 31))
    }
}

impl From<WidgetId> for AccessibilityId {
    fn from(id: WidgetId) -> Self {
        Self(id.0)
    }
}

/// 平台无关的语义角色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityRole {
    Window,
    Group,
    Button,
    CheckBox,
    Switch,
    TextField,
    StaticText,
    Tooltip,
    Slider,
    ScrollBar,
    List,
    ListItem,
    Tree,
    TreeItem,
    Menu,
    MenuItem,
    Dialog,
    Toolbar,
    Separator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityOrientation {
    Horizontal,
    Vertical,
}

/// 节点向辅助技术公布的动作集合。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityAction {
    Focus,
    Activate,
    Toggle,
    Increment,
    Decrement,
    SetValue,
}

/// 辅助技术发回 UI 的动作请求。
#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilityActionRequest {
    pub target: AccessibilityId,
    pub action: AccessibilityAction,
    pub value: Option<TextPayload>,
}

impl AccessibilityActionRequest {
    pub fn new(target: AccessibilityId, action: AccessibilityAction) -> Self {
        Self { target, action, value: None }
    }

    pub fn set_value(target: AccessibilityId, value: TextPayload) -> Self {
        Self { target, action: AccessibilityAction::SetValue, value: Some(value) }
    }
}

/// 语义节点的交互状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccessibilityState {
    pub disabled: bool,
    pub focused: bool,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
    pub read_only: bool,
    pub sensitive: bool,
}

/// 一棵语义子树的节点。
#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilityNode {
    pub id: AccessibilityId,
    pub role: AccessibilityRole,
    pub bounds: Rect,
    pub name: Option<String>,
    pub description: Option<String>,
    pub value: Option<String>,
    pub numeric_value: Option<f64>,
    pub numeric_minimum: Option<f64>,
    pub numeric_maximum: Option<f64>,
    pub orientation: Option<AccessibilityOrientation>,
    pub state: AccessibilityState,
    pub actions: Vec<AccessibilityAction>,
    pub labelled_by: Vec<AccessibilityId>,
    pub described_by: Vec<AccessibilityId>,
    pub children: Vec<AccessibilityNode>,
}

impl AccessibilityNode {
    pub fn new(id: AccessibilityId, role: AccessibilityRole, bounds: Rect) -> Self {
        Self {
            id,
            role,
            bounds,
            name: None,
            description: None,
            value: None,
            numeric_value: None,
            numeric_minimum: None,
            numeric_maximum: None,
            orientation: None,
            state: AccessibilityState::default(),
            actions: Vec::new(),
            labelled_by: Vec::new(),
            described_by: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn with_numeric_value(mut self, value: f64, minimum: f64, maximum: f64) -> Self {
        self.numeric_value = Some(value);
        self.numeric_minimum = Some(minimum);
        self.numeric_maximum = Some(maximum);
        self
    }

    pub fn with_orientation(mut self, orientation: AccessibilityOrientation) -> Self {
        self.orientation = Some(orientation);
        self
    }

    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.state.disabled = disabled;
        self
    }

    pub fn with_focused(mut self, focused: bool) -> Self {
        self.state.focused = focused;
        self
    }

    pub fn with_checked(mut self, checked: bool) -> Self {
        self.state.checked = Some(checked);
        self
    }

    pub fn with_selected(mut self, selected: bool) -> Self {
        self.state.selected = Some(selected);
        self
    }

    pub fn with_expanded(mut self, expanded: bool) -> Self {
        self.state.expanded = Some(expanded);
        self
    }

    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.state.read_only = read_only;
        self
    }

    pub fn with_sensitive(mut self, sensitive: bool) -> Self {
        self.state.sensitive = sensitive;
        self
    }

    pub fn with_action(mut self, action: AccessibilityAction) -> Self {
        if !self.actions.contains(&action) {
            self.actions.push(action);
        }
        self
    }

    pub fn with_labelled_by(mut self, id: AccessibilityId) -> Self {
        self.labelled_by.push(id);
        self
    }

    pub fn with_described_by(mut self, id: AccessibilityId) -> Self {
        self.described_by.push(id);
        self
    }

    pub fn with_child(mut self, child: AccessibilityNode) -> Self {
        self.children.push(child);
        self
    }
}

/// 将 widget 本地坐标转换为屏幕物理像素坐标的上下文。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AccessibilityContext {
    screen_offset_x: f32,
    screen_offset_y: f32,
}

impl AccessibilityContext {
    pub fn new(screen_offset_x: f32, screen_offset_y: f32) -> Self {
        Self { screen_offset_x, screen_offset_y }
    }

    pub fn offset_by(self, local_x: f32, local_y: f32) -> Self {
        Self::new(self.screen_offset_x + local_x, self.screen_offset_y + local_y)
    }

    pub fn screen_bounds(self, local_bounds: Rect) -> Rect {
        Rect::new(
            self.screen_offset_x + local_bounds.x,
            self.screen_offset_y + local_bounds.y,
            local_bounds.w,
            local_bounds.h,
        )
    }
}

/// 可访问性树校验失败的明确原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessibilityValidationError {
    DuplicateId(AccessibilityId),
    InvalidBounds(AccessibilityId),
    MissingFocusTarget(AccessibilityId),
    FocusTargetNotFocused(AccessibilityId),
    OrphanedFocusedNode(AccessibilityId),
    SensitiveValueExposed(AccessibilityId),
    InvalidNumericValue(AccessibilityId),
    MissingNumericValue(AccessibilityId),
    MissingOrientation(AccessibilityId),
}

/// 一次平台更新所需的完整语义树和唯一焦点。
#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilityTree {
    pub root: AccessibilityNode,
    pub focus: Option<AccessibilityId>,
}

impl AccessibilityTree {
    pub fn new(root: AccessibilityNode, focus: Option<AccessibilityId>) -> Self {
        Self { root, focus }
    }

    pub fn validate(&self) -> Result<(), Vec<AccessibilityValidationError>> {
        let mut ids = HashSet::new();
        let mut focused_ids = Vec::new();
        let mut errors = Vec::new();
        Self::validate_node(&self.root, &mut ids, &mut focused_ids, &mut errors);

        match self.focus {
            Some(focus_id) if !ids.contains(&focus_id) => {
                errors.push(AccessibilityValidationError::MissingFocusTarget(focus_id));
            }
            Some(focus_id) => {
                if !focused_ids.contains(&focus_id) {
                    errors.push(AccessibilityValidationError::FocusTargetNotFocused(focus_id));
                }
                for focused_id in focused_ids {
                    if focused_id != focus_id {
                        errors.push(AccessibilityValidationError::OrphanedFocusedNode(focused_id));
                    }
                }
            }
            None => {
                errors.extend(
                    focused_ids.into_iter().map(AccessibilityValidationError::OrphanedFocusedNode),
                );
            }
        }

        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }

    fn validate_node(
        node: &AccessibilityNode,
        ids: &mut HashSet<AccessibilityId>,
        focused_ids: &mut Vec<AccessibilityId>,
        errors: &mut Vec<AccessibilityValidationError>,
    ) {
        if !ids.insert(node.id) {
            errors.push(AccessibilityValidationError::DuplicateId(node.id));
        }
        if !valid_bounds(node.bounds) {
            errors.push(AccessibilityValidationError::InvalidBounds(node.id));
        }
        if node.state.focused {
            focused_ids.push(node.id);
        }
        if node.state.sensitive && node.value.is_some() {
            errors.push(AccessibilityValidationError::SensitiveValueExposed(node.id));
        }
        if !valid_numeric_value(node) {
            errors.push(AccessibilityValidationError::InvalidNumericValue(node.id));
        }
        if matches!(node.role, AccessibilityRole::Slider | AccessibilityRole::ScrollBar) {
            if node.numeric_value.is_none()
                && node.numeric_minimum.is_none()
                && node.numeric_maximum.is_none()
            {
                errors.push(AccessibilityValidationError::MissingNumericValue(node.id));
            }
            if node.orientation.is_none() {
                errors.push(AccessibilityValidationError::MissingOrientation(node.id));
            }
        }
        for child in &node.children {
            Self::validate_node(child, ids, focused_ids, errors);
        }
    }
}

fn valid_bounds(bounds: Rect) -> bool {
    bounds.x.is_finite()
        && bounds.y.is_finite()
        && bounds.w.is_finite()
        && bounds.h.is_finite()
        && bounds.w > 0.0
        && bounds.h > 0.0
}

fn valid_numeric_value(node: &AccessibilityNode) -> bool {
    match (node.numeric_value, node.numeric_minimum, node.numeric_maximum) {
        (None, None, None) => true,
        (Some(value), Some(minimum), Some(maximum)) => {
            value.is_finite()
                && minimum.is_finite()
                && maximum.is_finite()
                && minimum <= value
                && value <= maximum
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::widget::WidgetId;

    fn named_button(id: u64, bounds: Rect) -> AccessibilityNode {
        AccessibilityNode::new(AccessibilityId(id), AccessibilityRole::Button, bounds)
            .with_name("保存")
            .with_action(AccessibilityAction::Activate)
    }

    #[test]
    fn widget_identity_maps_to_stable_accessibility_identity() {
        assert_eq!(AccessibilityId::from(WidgetId(42)), AccessibilityId(42));
    }

    #[test]
    fn context_applies_container_offset_exactly_once() {
        let root_context = AccessibilityContext::new(100.0, 200.0);
        let child_context = root_context.offset_by(10.0, 20.0);

        assert_eq!(
            child_context.screen_bounds(Rect::new(5.0, 6.0, 30.0, 40.0)),
            Rect::new(115.0, 226.0, 30.0, 40.0)
        );
    }

    #[test]
    fn tree_rejects_duplicate_ids() {
        let root = AccessibilityNode::new(
            AccessibilityId(1),
            AccessibilityRole::Group,
            Rect::new(0.0, 0.0, 100.0, 100.0),
        )
        .with_child(named_button(2, Rect::new(0.0, 0.0, 20.0, 20.0)))
        .with_child(named_button(2, Rect::new(30.0, 0.0, 20.0, 20.0)));

        assert_eq!(
            AccessibilityTree::new(root, None).validate(),
            Err(vec![AccessibilityValidationError::DuplicateId(AccessibilityId(2))])
        );
    }

    #[test]
    fn tree_rejects_non_finite_or_empty_bounds() {
        let root = AccessibilityNode::new(
            AccessibilityId(1),
            AccessibilityRole::Group,
            Rect::new(f32::NAN, 0.0, 0.0, 10.0),
        );

        assert_eq!(
            AccessibilityTree::new(root, None).validate(),
            Err(vec![AccessibilityValidationError::InvalidBounds(AccessibilityId(1))])
        );
    }

    #[test]
    fn tree_rejects_focused_node_without_matching_tree_focus() {
        let root = named_button(1, Rect::new(0.0, 0.0, 20.0, 20.0)).with_focused(true);

        assert_eq!(
            AccessibilityTree::new(root, None).validate(),
            Err(vec![AccessibilityValidationError::OrphanedFocusedNode(AccessibilityId(1))])
        );
    }

    #[test]
    fn tree_rejects_missing_focus_target() {
        let root = named_button(1, Rect::new(0.0, 0.0, 20.0, 20.0));

        assert_eq!(
            AccessibilityTree::new(root, Some(AccessibilityId(9))).validate(),
            Err(vec![AccessibilityValidationError::MissingFocusTarget(AccessibilityId(9))])
        );
    }

    #[test]
    fn tree_rejects_focus_target_without_focused_state() {
        let root = named_button(1, Rect::new(0.0, 0.0, 20.0, 20.0));

        assert_eq!(
            AccessibilityTree::new(root, Some(AccessibilityId(1))).validate(),
            Err(vec![AccessibilityValidationError::FocusTargetNotFocused(AccessibilityId(1))])
        );
    }

    #[test]
    fn tree_rejects_sensitive_text_value_exposure() {
        let root = AccessibilityNode::new(
            AccessibilityId(1),
            AccessibilityRole::TextField,
            Rect::new(0.0, 0.0, 100.0, 24.0),
        )
        .with_sensitive(true)
        .with_value("secret");

        assert_eq!(
            AccessibilityTree::new(root, None).validate(),
            Err(vec![AccessibilityValidationError::SensitiveValueExposed(AccessibilityId(1))])
        );
    }

    #[test]
    fn tree_rejects_incomplete_or_out_of_range_numeric_values() {
        let root =
            named_button(1, Rect::new(0.0, 0.0, 20.0, 20.0)).with_numeric_value(12.0, 0.0, 10.0);

        assert_eq!(
            AccessibilityTree::new(root, None).validate(),
            Err(vec![AccessibilityValidationError::InvalidNumericValue(AccessibilityId(1))])
        );
    }

    #[test]
    fn tree_requires_orientation_for_slider_semantics() {
        let root = AccessibilityNode::new(
            AccessibilityId(1),
            AccessibilityRole::Slider,
            Rect::new(0.0, 0.0, 20.0, 100.0),
        )
        .with_numeric_value(5.0, 0.0, 10.0);

        assert_eq!(
            AccessibilityTree::new(root, None).validate(),
            Err(vec![AccessibilityValidationError::MissingOrientation(AccessibilityId(1))])
        );
    }

    #[test]
    fn tree_requires_numeric_value_for_scrollbar_semantics() {
        let root = AccessibilityNode::new(
            AccessibilityId(1),
            AccessibilityRole::ScrollBar,
            Rect::new(0.0, 0.0, 20.0, 100.0),
        )
        .with_orientation(AccessibilityOrientation::Vertical);

        assert_eq!(
            AccessibilityTree::new(root, None).validate(),
            Err(vec![AccessibilityValidationError::MissingNumericValue(AccessibilityId(1))])
        );
    }
}
