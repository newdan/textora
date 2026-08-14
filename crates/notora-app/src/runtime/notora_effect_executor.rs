use crate::action::{
    CardQuery, DocumentLoadRequest, MetadataMutation, NoteCreationTarget, NotoraAction,
    NotoraEffect, SaveConflictRequest, TrashOperation, WorkspaceTransitionRequest,
};
use crate::effect_executor::ExternalOpenRequest;

/// Notora effect 解释器所需的产品能力。
pub(super) trait NotoraEffectTarget {
    fn query_cards(&mut self, query: CardQuery) -> Vec<NotoraAction>;
    fn request_note_creation(
        &mut self,
        kind: notora_core::DocumentKind,
        target: NoteCreationTarget,
    ) -> Vec<NotoraAction>;
    fn execute_note_command(&mut self, command: notora_core::note_command::NoteCommand);
    fn execute_directory_command(&mut self, command: notora_core::WorkspaceDirectoryCommand);
    fn choose_workspace_creation_location(&mut self) -> Vec<NotoraAction>;
    fn prepare_workspace_transition(&mut self, request: WorkspaceTransitionRequest);
    fn commit_title(&mut self, title: String);
    fn toggle_editor_view(&mut self);
    fn toggle_mindmap_style_panel(&mut self);
    fn dispatch_mindmap_style_panel(&mut self, action: ui::core::widget::MindmapStylePanelAction);
    fn execute_semantic_edit(&mut self, command: ui::plugin::SemanticEditCommand);
    fn execute_metadata_mutation(&mut self, mutation: MetadataMutation) -> Vec<NotoraAction>;
    fn execute_trash_operation(&mut self, operation: TrashOperation);
    fn choose_note_move_directory(&mut self, note_id: notora_core::NoteId) -> Vec<NotoraAction>;
    fn prepare_document(&mut self, request: DocumentLoadRequest) -> Vec<NotoraAction>;
    fn promote_active_preview(&mut self);
    fn choose_workspace_root(&mut self);
    fn open_external_files(&mut self, request: ExternalOpenRequest);
    fn create_untitled_external(&mut self, kind: notora_core::DocumentKind) -> Vec<NotoraAction>;
    fn resolve_save_conflict(&mut self, request: SaveConflictRequest);
    fn apply_product_settings_update(
        &mut self,
        update: crate::settings_overlay::ProductSettingsUpdate,
    );
    fn persist_product_settings(&mut self);
    fn persist_layout(&mut self);
}

pub(super) struct NotoraEffectExecutor;

impl NotoraEffectExecutor {
    pub(super) fn execute<T: NotoraEffectTarget>(
        target: &mut T,
        effect: NotoraEffect,
    ) -> Vec<NotoraAction> {
        match effect {
            NotoraEffect::QueryCards(query) => target.query_cards(query),
            NotoraEffect::RequestNoteCreation { kind, target: creation_target } => {
                target.request_note_creation(kind, creation_target)
            }
            NotoraEffect::ExecuteNoteCommand(command) => {
                target.execute_note_command(command);
                Vec::new()
            }
            NotoraEffect::ExecuteDirectoryCommand(command) => {
                target.execute_directory_command(command);
                Vec::new()
            }
            NotoraEffect::ChooseWorkspaceCreationLocation => {
                target.choose_workspace_creation_location()
            }
            NotoraEffect::PrepareWorkspaceTransition(request) => {
                target.prepare_workspace_transition(request);
                Vec::new()
            }
            NotoraEffect::CommitTitle(title) => {
                target.commit_title(title);
                Vec::new()
            }
            NotoraEffect::ToggleEditorView => {
                target.toggle_editor_view();
                Vec::new()
            }
            NotoraEffect::ToggleMindmapStylePanel => {
                target.toggle_mindmap_style_panel();
                Vec::new()
            }
            NotoraEffect::DispatchMindmapStylePanel(action) => {
                target.dispatch_mindmap_style_panel(action);
                Vec::new()
            }
            NotoraEffect::ExecuteSemanticEdit(command) => {
                target.execute_semantic_edit(command);
                Vec::new()
            }
            NotoraEffect::ExecuteMetadataMutation(mutation) => {
                target.execute_metadata_mutation(mutation)
            }
            NotoraEffect::ExecuteTrashOperation(operation) => {
                target.execute_trash_operation(operation);
                Vec::new()
            }
            NotoraEffect::ChooseNoteMoveDirectory(note_id) => {
                target.choose_note_move_directory(note_id)
            }
            NotoraEffect::PrepareDocument(request) => target.prepare_document(request),
            NotoraEffect::PromoteActivePreview => {
                target.promote_active_preview();
                Vec::new()
            }
            NotoraEffect::ChooseWorkspaceRoot => {
                target.choose_workspace_root();
                Vec::new()
            }
            NotoraEffect::OpenExternalFiles(request) => {
                target.open_external_files(request);
                Vec::new()
            }
            NotoraEffect::CreateUntitledExternal(kind) => target.create_untitled_external(kind),
            NotoraEffect::ResolveSaveConflict(request) => {
                target.resolve_save_conflict(request);
                Vec::new()
            }
            NotoraEffect::ApplyProductSettingsUpdate(update) => {
                target.apply_product_settings_update(update);
                Vec::new()
            }
            NotoraEffect::PersistProductSettings => {
                target.persist_product_settings();
                Vec::new()
            }
            NotoraEffect::PersistLayout => {
                target.persist_layout();
                Vec::new()
            }
            NotoraEffect::Redraw => unreachable!("redraw is handled by EffectExecutor"),
        }
    }
}
