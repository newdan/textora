use crate::action::NotoraAction;
use crate::product::PersistenceCompletion;

use super::product_event_coordinator::PersistenceCompletionTarget;

pub(super) struct PersistenceCompletionInterpreter;

impl PersistenceCompletionInterpreter {
    pub(super) fn apply<T: PersistenceCompletionTarget>(
        target: &mut T,
        completion: PersistenceCompletion,
    ) {
        match completion {
            PersistenceCompletion::SettingsPersistenceCompleted { result } => {
                target.record_settings_persistence_result(result);
            }
            PersistenceCompletion::SessionPersistenceFailed { message } => {
                target.dispatch_action(NotoraAction::NoteCommandFailed(message));
            }
        }
    }
}
