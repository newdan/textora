use crate::action::NotoraAction;
use crate::product::DocumentCompletion;

use super::product_event_coordinator::DocumentCompletionTarget;

pub(super) struct DocumentCompletionInterpreter;

impl DocumentCompletionInterpreter {
    pub(super) fn apply<T: DocumentCompletionTarget>(
        target: &mut T,
        completion: DocumentCompletion,
    ) {
        match completion {
            DocumentCompletion::ExternalFileOpenCompleted {
                canonical_path,
                document,
                activate,
            } => target.complete_external_file_open(canonical_path, document, activate),
            DocumentCompletion::ExternalFileOpenFailed { message } => {
                target.dispatch_action(NotoraAction::NoteCommandFailed(message));
            }
            DocumentCompletion::ExternalDocumentLoaded { request, document } => {
                target.install_loaded_preview(request, document);
            }
            DocumentCompletion::ExternalDocumentLoadFailed { request, message }
                if target.selection_matches(request) =>
            {
                target.dispatch_action(NotoraAction::NoteCommandFailed(message));
            }
            DocumentCompletion::ExternalDocumentLoadFailed { .. } => {}
            DocumentCompletion::ExternalSaveAsCanonicalized {
                tab_id,
                external_file_id,
                content_revision,
                result,
            } => target.complete_external_save_as_canonicalization(
                tab_id,
                external_file_id,
                content_revision,
                result,
            ),
            DocumentCompletion::ConflictReloadCompleted {
                identity,
                tab_id,
                content_revision,
                document,
            } => target.complete_conflict_reload(identity, tab_id, content_revision, document),
            DocumentCompletion::ConflictReloadFailed { identity, message } => {
                if target.active_save_conflict_identity() == Some(identity) {
                    target.dispatch_action(NotoraAction::NoteCommandFailed(message));
                }
            }
            DocumentCompletion::ConflictReloadRequiresUnlock { identity, tab_id } => {
                if target.active_save_conflict_identity() == Some(identity) {
                    target.relock_conflicted_document(identity, tab_id);
                }
            }
            DocumentCompletion::ConflictRetryRevisionCaptured {
                identity,
                tab_id,
                content_revision,
                path,
                disk_revision,
            } => target.complete_conflict_retry_revision_capture(
                identity,
                tab_id,
                content_revision,
                path,
                disk_revision,
            ),
            DocumentCompletion::ConflictRetryRevisionFailed { identity, message } => {
                if target.active_save_conflict_identity() == Some(identity) {
                    target.dispatch_action(NotoraAction::NoteCommandFailed(message));
                }
            }
        }
    }
}
