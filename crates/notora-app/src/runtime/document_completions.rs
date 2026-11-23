use crate::action::NotoraAction;
use crate::product::DocumentCompletion;

use super::NotoraRuntime;

impl NotoraRuntime {
    pub(super) fn apply_document_completion(&mut self, completion: DocumentCompletion) {
        match completion {
            DocumentCompletion::SystemOpenPathsRequested { paths } => {
                self.dispatch_action(NotoraAction::ExternalPathsReceived(paths));
            }
            DocumentCompletion::ExternalFileOpenCompleted {
                canonical_path,
                document,
                activate,
            } => self.complete_external_file_open(canonical_path, document, activate),
            DocumentCompletion::ExternalFileOpenFailed { message } => {
                self.dispatch_action(NotoraAction::NoteCommandFailed(message));
            }
            DocumentCompletion::ExternalDocumentLoaded { request, document } => {
                self.install_loaded_preview(request, document);
            }
            DocumentCompletion::ExternalDocumentLoadFailed { request, message }
                if self.selection_matches(request) =>
            {
                self.dispatch_action(NotoraAction::NoteCommandFailed(message));
            }
            DocumentCompletion::ExternalDocumentLoadFailed { .. } => {}
            DocumentCompletion::ExternalSaveAsCanonicalized {
                tab_id,
                external_file_id,
                content_revision,
                result,
            } => {
                let outcome = self.document_runtime.complete_external_save_as_canonicalization(
                    tab_id,
                    external_file_id,
                    content_revision,
                    result,
                );
                self.apply_document_outcome(outcome);
            }
            DocumentCompletion::ConflictReloadCompleted {
                identity,
                tab_id,
                content_revision,
                document,
            } => {
                let outcome = self.document_runtime.complete_conflict_reload(
                    identity,
                    tab_id,
                    content_revision,
                    document,
                );
                self.apply_document_outcome(outcome);
            }
            DocumentCompletion::ConflictReloadFailed { identity, message }
            | DocumentCompletion::ConflictRetryRevisionFailed { identity, message }
                if self.action_runtime.has_save_conflict(identity) =>
            {
                self.dispatch_action(NotoraAction::NoteCommandFailed(message));
            }
            DocumentCompletion::ConflictReloadRequiresUnlock { identity, tab_id }
                if self.action_runtime.has_save_conflict(identity) =>
            {
                self.relock_conflicted_document(identity, tab_id);
            }
            DocumentCompletion::ConflictReloadFailed { .. }
            | DocumentCompletion::ConflictRetryRevisionFailed { .. }
            | DocumentCompletion::ConflictReloadRequiresUnlock { .. } => {}
            DocumentCompletion::ConflictRetryRevisionCaptured {
                identity,
                tab_id,
                content_revision,
                path,
                disk_revision,
            } => {
                let request = self.manual_save_request_for_tab(tab_id);
                let outcome = self.document_runtime.complete_conflict_retry_revision_capture(
                    identity,
                    tab_id,
                    content_revision,
                    path,
                    disk_revision,
                    request,
                );
                self.apply_document_outcome(outcome);
            }
        }
    }
}
