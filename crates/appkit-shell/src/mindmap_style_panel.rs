#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MindmapStylePanelSession {
    Closed,
    Open { presets_expanded: bool },
}

impl MindmapStylePanelSession {
    pub fn is_visible(self) -> bool {
        matches!(self, Self::Open { .. })
    }

    pub fn presets_expanded(self) -> bool {
        match self {
            Self::Closed => false,
            Self::Open { presets_expanded } => presets_expanded,
        }
    }

    pub fn toggle_visibility(&mut self) {
        *self = match self {
            Self::Closed => Self::Open { presets_expanded: true },
            Self::Open { .. } => Self::Closed,
        };
    }

    pub fn close(&mut self) {
        *self = Self::Closed;
    }

    pub fn toggle_presets(&mut self) {
        let Self::Open { presets_expanded } = self else {
            return;
        };
        *presets_expanded = !*presets_expanded;
    }
}

#[cfg(test)]
mod tests {
    use super::MindmapStylePanelSession;

    #[test]
    fn style_panel_session_opens_expanded_and_closes_without_persistence() {
        let mut session = MindmapStylePanelSession::Closed;

        session.toggle_visibility();
        assert_eq!(session, MindmapStylePanelSession::Open { presets_expanded: true });

        session.toggle_presets();
        assert_eq!(session, MindmapStylePanelSession::Open { presets_expanded: false });

        session.close();
        assert_eq!(session, MindmapStylePanelSession::Closed);
    }
}
