use appkit_shell::{ShellEffect, ShellEffectStep};

/// Notora 组合层为通用 shell effect 提供的能力端口。
pub(crate) trait ShellEffectTarget {
    fn invalidate_reshape(&mut self);

    fn synchronize_window_chrome(&mut self) {}

    fn update_window_title(&mut self);

    fn persist_settings(&mut self);

    fn persist_workspace(&mut self);

    fn request_redraw(&mut self);
}

pub(crate) struct ShellEffectExecutor;

impl ShellEffectExecutor {
    pub(crate) fn execute(target: &mut impl ShellEffectTarget, effect: ShellEffect) {
        for step in effect.steps() {
            match step {
                ShellEffectStep::Reshape => target.invalidate_reshape(),
                ShellEffectStep::SyncWindowChrome => target.synchronize_window_chrome(),
                ShellEffectStep::UpdateTitle => target.update_window_title(),
                ShellEffectStep::PersistSettings => target.persist_settings(),
                ShellEffectStep::PersistWorkspace => target.persist_workspace(),
                ShellEffectStep::Redraw => target.request_redraw(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ShellEffectExecutor, ShellEffectTarget};
    use appkit_shell::ShellEffect;

    #[derive(Default)]
    struct Recorder {
        steps: Vec<&'static str>,
    }

    impl ShellEffectTarget for Recorder {
        fn invalidate_reshape(&mut self) {
            self.steps.push("reshape");
        }

        fn synchronize_window_chrome(&mut self) {
            self.steps.push("window-chrome");
        }

        fn update_window_title(&mut self) {
            self.steps.push("title");
        }

        fn persist_settings(&mut self) {
            self.steps.push("settings");
        }

        fn persist_workspace(&mut self) {
            self.steps.push("workspace");
        }

        fn request_redraw(&mut self) {
            self.steps.push("redraw");
        }
    }

    #[test]
    fn executor_honors_the_shared_shell_effect_order() {
        let effect = ShellEffect::RESHAPE
            .merge(ShellEffect::SYNC_WINDOW_CHROME)
            .merge(ShellEffect::UPDATE_TITLE)
            .merge(ShellEffect::PERSIST_SETTINGS)
            .merge(ShellEffect::PERSIST_WORKSPACE);
        let mut recorder = Recorder::default();

        ShellEffectExecutor::execute(&mut recorder, effect);

        assert_eq!(
            recorder.steps,
            vec!["reshape", "window-chrome", "title", "settings", "workspace", "redraw"]
        );
    }
}
