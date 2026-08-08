use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum ShellEvent {
    StartBackgroundServices,
    ReshapeResultsReady,
    FileSafetyResultsReady,
    SaveResultsReady,
    ProductWake,
    Accessibility(Arc<accesskit_winit::Event>),
}

impl From<accesskit_winit::Event> for ShellEvent {
    fn from(event: accesskit_winit::Event) -> Self {
        Self::Accessibility(Arc::new(event))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellEffectStep {
    Reshape,
    SyncWindowChrome,
    UpdateTitle,
    PersistSettings,
    PersistWorkspace,
    Redraw,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ShellEffect {
    pub redraw: bool,
    pub reshape: bool,
    pub update_title: bool,
    pub persist_workspace: bool,
    pub persist_settings: bool,
    pub sync_window_chrome: bool,
}

impl ShellEffect {
    pub const NONE: Self = Self {
        redraw: false,
        reshape: false,
        update_title: false,
        persist_workspace: false,
        persist_settings: false,
        sync_window_chrome: false,
    };
    pub const REDRAW: Self = Self { redraw: true, ..Self::NONE };
    pub const RESHAPE: Self = Self { redraw: true, reshape: true, ..Self::NONE };
    pub const UPDATE_TITLE: Self = Self { redraw: true, update_title: true, ..Self::NONE };
    pub const PERSIST_WORKSPACE: Self = Self { persist_workspace: true, ..Self::NONE };
    pub const PERSIST_SETTINGS: Self = Self { persist_settings: true, ..Self::NONE };
    pub const SYNC_WINDOW_CHROME: Self =
        Self { redraw: true, sync_window_chrome: true, ..Self::NONE };

    pub const fn merge(self, other: Self) -> Self {
        Self {
            redraw: self.redraw || other.redraw,
            reshape: self.reshape || other.reshape,
            update_title: self.update_title || other.update_title,
            persist_workspace: self.persist_workspace || other.persist_workspace,
            persist_settings: self.persist_settings || other.persist_settings,
            sync_window_chrome: self.sync_window_chrome || other.sync_window_chrome,
        }
    }

    pub fn steps(self) -> impl Iterator<Item = ShellEffectStep> {
        [
            self.reshape.then_some(ShellEffectStep::Reshape),
            self.sync_window_chrome.then_some(ShellEffectStep::SyncWindowChrome),
            self.update_title.then_some(ShellEffectStep::UpdateTitle),
            self.persist_settings.then_some(ShellEffectStep::PersistSettings),
            self.persist_workspace.then_some(ShellEffectStep::PersistWorkspace),
            self.redraw.then_some(ShellEffectStep::Redraw),
        ]
        .into_iter()
        .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_wake_carries_no_payload() {
        assert!(matches!(ShellEvent::ProductWake, ShellEvent::ProductWake));
    }

    #[test]
    fn merge_obeys_boolean_union_laws() {
        let x = ShellEffect::RESHAPE.merge(ShellEffect::PERSIST_SETTINGS);
        let y = ShellEffect::UPDATE_TITLE.merge(ShellEffect::PERSIST_WORKSPACE);
        let z = ShellEffect::SYNC_WINDOW_CHROME;

        assert_eq!(x.merge(ShellEffect::NONE), x);
        assert_eq!(x.merge(x), x);
        assert_eq!(x.merge(y), y.merge(x));
        assert_eq!(x.merge(y).merge(z), x.merge(y.merge(z)));
    }

    #[test]
    fn execution_steps_have_fixed_order() {
        let effect = ShellEffect::RESHAPE
            .merge(ShellEffect::SYNC_WINDOW_CHROME)
            .merge(ShellEffect::UPDATE_TITLE)
            .merge(ShellEffect::PERSIST_SETTINGS)
            .merge(ShellEffect::PERSIST_WORKSPACE);

        assert_eq!(
            effect.steps().collect::<Vec<_>>(),
            vec![
                ShellEffectStep::Reshape,
                ShellEffectStep::SyncWindowChrome,
                ShellEffectStep::UpdateTitle,
                ShellEffectStep::PersistSettings,
                ShellEffectStep::PersistWorkspace,
                ShellEffectStep::Redraw,
            ]
        );
    }
}
