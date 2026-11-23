use std::time::Instant;

use super::{NotoraRuntime, action_requires_session_persistence};
use crate::action::NotoraAction;
use crate::autosave::AutoSaveState;

#[test]
fn session_restore_related_user_actions_are_debounced_for_persistence() {
    assert!(action_requires_session_persistence(&NotoraAction::NavigationSelected(
        notora_core::NavigationScope::Starred
    )));
    assert!(action_requires_session_persistence(&NotoraAction::WorkspaceRootExpansionToggled));
    assert!(action_requires_session_persistence(&NotoraAction::TagRootExpansionToggled));
    assert!(action_requires_session_persistence(&NotoraAction::CardSelected(
        notora_core::DocumentIdentity::Note(notora_core::NoteId::generate())
    )));
    assert!(!action_requires_session_persistence(&NotoraAction::CardListScrolled {
        offset_px: 5.0,
        near_end: false
    }));
}

#[test]
fn editor_save_status_is_derived_from_typed_autosave_state() {
    let deadline = Instant::now();
    assert_eq!(
        NotoraRuntime::editor_save_status(
            Some(AutoSaveState::Saving { content_revision: 3 }),
            true,
            None,
        ),
        "保存中"
    );
    assert_eq!(
        NotoraRuntime::editor_save_status(
            Some(AutoSaveState::Failed { content_revision: 3 }),
            true,
            None,
        ),
        "保存失败"
    );
    assert_eq!(
        NotoraRuntime::editor_save_status(
            Some(AutoSaveState::Scheduled { deadline, content_revision: 3 }),
            true,
            None,
        ),
        "待保存"
    );
    assert_eq!(NotoraRuntime::editor_save_status(None, true, None), "未保存");
    assert_eq!(NotoraRuntime::editor_save_status(None, false, None), "已保存");
}

#[test]
fn editor_save_status_includes_the_failure_reason() {
    assert_eq!(
        NotoraRuntime::editor_save_status(
            Some(AutoSaveState::Failed { content_revision: 3 }),
            true,
            Some("file is read-only"),
        ),
        "保存失败：file is read-only"
    );
}
