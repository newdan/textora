//! notora reducer effect 的顺序执行协议。

use std::path::PathBuf;

use appkit_core::workspace::types::TabId;
use appkit_shell::ShellEffect;

use crate::action::{NotoraAction, NotoraEffect};

/// 外部打开来源最终统一为同一个 effect；路径来源不应拥有单独的验证逻辑。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalOpenRequest {
    ShowFileDialog,
    Paths(Vec<PathBuf>),
}

/// 用户显式保存当前文档时已经由产品判定好的来源类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManualSaveRequest {
    Note { tab_id: TabId, content_revision: u64 },
    ExistingExternalFile { tab_id: TabId },
    UntitledExternalFile { tab_id: TabId, external_file_id: notora_core::ExternalFileId },
}

/// 一个 reducer effect 完成后交还 ActionRuntime 的显式结果。
#[derive(Debug, Default, PartialEq)]
pub(crate) struct EffectExecution {
    pub(crate) shell_effect: ShellEffect,
    pub(crate) follow_up_actions: Vec<NotoraAction>,
}

impl EffectExecution {
    fn new(shell_effect: ShellEffect, follow_up_actions: Vec<NotoraAction>) -> Self {
        Self { shell_effect, follow_up_actions }
    }
}

/// 保留 reducer effect 的声明顺序，并把业务执行与 follow-up 入队分开。
pub(crate) struct EffectExecutor;

impl EffectExecutor {
    pub(crate) fn execute(
        effect: NotoraEffect,
        execute_operation: impl FnOnce(NotoraEffect) -> Vec<NotoraAction>,
    ) -> EffectExecution {
        if matches!(effect, NotoraEffect::Redraw) {
            return EffectExecution::new(ShellEffect::REDRAW, Vec::new());
        }
        let shell_effect = if matches!(effect, NotoraEffect::ToggleEditorView) {
            ShellEffect::REDRAW
        } else {
            ShellEffect::NONE
        };
        let follow_up_actions = execute_operation(effect);
        EffectExecution::new(shell_effect, follow_up_actions)
    }
}

#[cfg(test)]
mod tests {
    use crate::action::{CardQuery, NotoraAction, NotoraEffect};
    use notora_core::NavigationScope;

    use super::EffectExecutor;

    #[test]
    fn executor_routes_a_typed_operation_and_returns_its_follow_up_actions() {
        let expected_query = CardQuery::from(NavigationScope::Starred);
        let execution =
            EffectExecutor::execute(NotoraEffect::QueryCards(expected_query.clone()), |effect| {
                match effect {
                    NotoraEffect::QueryCards(query) => {
                        assert_eq!(query, expected_query);
                        vec![NotoraAction::CardQueryFailed { query, message: "offline".to_owned() }]
                    }
                    _ => panic!("executor should preserve the typed effect"),
                }
            });

        assert_eq!(execution.shell_effect, appkit_shell::ShellEffect::NONE);
        assert!(matches!(
            execution.follow_up_actions.as_slice(),
            [NotoraAction::CardQueryFailed { message, .. }] if message == "offline"
        ));
    }

    #[test]
    fn redraw_is_a_shell_only_effect() {
        let execution = EffectExecutor::execute(NotoraEffect::Redraw, |_| {
            panic!("redraw should not invoke a product operation")
        });

        assert_eq!(execution.shell_effect, appkit_shell::ShellEffect::REDRAW);
        assert!(execution.follow_up_actions.is_empty());
    }

    #[test]
    fn editor_view_toggle_keeps_its_redraw_effect() {
        let mut toggled = false;
        let execution = EffectExecutor::execute(NotoraEffect::ToggleEditorView, |effect| {
            assert!(matches!(effect, NotoraEffect::ToggleEditorView));
            toggled = true;
            Vec::new()
        });

        assert!(toggled);
        assert_eq!(execution.shell_effect, appkit_shell::ShellEffect::REDRAW);
    }
}
