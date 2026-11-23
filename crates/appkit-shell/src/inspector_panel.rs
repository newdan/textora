//! 属性面板的标签页会话与事件来源。

use std::sync::Arc;

use appkit_core::tab_set::types::TabId;
use ui::inspector_panel::{
    InspectorPanelAction, InspectorPanelDescriptor, InspectorPanelPresentation, PanelId,
};

#[derive(Clone, Debug)]
struct PanelMount(Arc<()>);

impl Default for PanelMount {
    fn default() -> Self {
        Self(Arc::new(()))
    }
}

impl PartialEq for PanelMount {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for PanelMount {}

#[derive(Debug, Default)]
pub(crate) enum InspectorPanelSession {
    #[default]
    Closed,
    Open(OpenInspectorPanel),
}

#[derive(Debug, Default)]
pub(crate) struct OpenInspectorPanel {
    mount: PanelMount,
    pub presentation: InspectorPanelPresentation,
}

impl InspectorPanelSession {
    pub(crate) fn toggle(&mut self) {
        *self = match self {
            Self::Closed => Self::Open(OpenInspectorPanel::default()),
            Self::Open(_) => Self::Closed,
        };
    }

    /// 切换视图或重新激活标签时，旧挂载产生的事件失效。
    pub(crate) fn renew_mount(&mut self) {
        if let Self::Open(panel) = self {
            panel.mount = PanelMount::default();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectorPanelStamp {
    pub(crate) tab_id: TabId,
    pub(crate) generation: u32,
    content_revision: u64,
    mount: PanelMount,
    panel_id: PanelId,
}

impl InspectorPanelStamp {
    /// 使用接收控件事件的布局所携带的 stamp。
    pub fn stamp(&self, action: InspectorPanelAction) -> Option<StampedInspectorPanelAction> {
        (self.panel_id == action.panel_id)
            .then(|| StampedInspectorPanelAction { stamp: self.clone(), action })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StampedInspectorPanelAction {
    pub(crate) stamp: InspectorPanelStamp,
    pub(crate) action: InspectorPanelAction,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MountedInspectorPanel {
    pub descriptor: InspectorPanelDescriptor,
    pub presentation: InspectorPanelPresentation,
    pub stamp: InspectorPanelStamp,
}

impl OpenInspectorPanel {
    pub(crate) fn stamp(
        &self,
        tab_id: TabId,
        generation: u32,
        content_revision: u64,
        panel_id: PanelId,
    ) -> InspectorPanelStamp {
        InspectorPanelStamp {
            tab_id,
            generation,
            content_revision,
            mount: self.mount.clone(),
            panel_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_core::tab_set::types::TabIdAllocator;

    #[test]
    fn mount_renewal_invalidates_old_events_without_clearing_presentation() {
        let tab_id = TabIdAllocator::new().allocate();
        let mut session = InspectorPanelSession::default();
        session.toggle();
        let InspectorPanelSession::Open(panel) = &mut session else {
            panic!("panel should be open");
        };
        panel.presentation.presets_expanded = false;
        let prior = panel.stamp(tab_id, 0, 0, PanelId("appearance".into()));

        session.renew_mount();

        let InspectorPanelSession::Open(panel) = session else {
            panic!("panel should remain open");
        };
        let current = panel.stamp(tab_id, 0, 0, PanelId("appearance".into()));
        assert_ne!(prior, current);
        assert!(!panel.presentation.presets_expanded);
    }
}
