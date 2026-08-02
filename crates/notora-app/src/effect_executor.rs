//! notora effect executor boundary.

use appkit_core::workspace::types::TabId;
use appkit_shell::ShellEffect;
use std::path::PathBuf;

use crate::action::{
    CardQuery, DocumentLoadRequest, MetadataMutation, NoteCreationTarget, NotoraEffect,
    SaveConflictRequest, TrashOperation,
};
use crate::settings_overlay::ProductSettingsUpdate;
use notora_core::DocumentKind;
use notora_core::note_command::NoteCommand;

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

/// 产品层外部能力的唯一入口。实现者可调度 worker、dialog、catalog 或 runtime。
pub trait NotoraEffectService {
    fn query_cards(&mut self, query: CardQuery);
    fn request_note_creation(&mut self, kind: DocumentKind, target: NoteCreationTarget);
    fn execute_note_command(&mut self, _command: NoteCommand) {}
    fn execute_metadata_mutation(&mut self, _mutation: MetadataMutation) {}
    fn execute_trash_operation(&mut self, _operation: TrashOperation) {}
    fn choose_note_rename_destination(&mut self, _note_id: notora_core::NoteId) {}
    fn choose_note_move_directory(&mut self, _note_id: notora_core::NoteId) {}
    fn prepare_document(&mut self, request: DocumentLoadRequest);
    fn promote_active_preview(&mut self) {}
    fn open_external_files(&mut self, _request: ExternalOpenRequest) {}
    fn create_untitled_external(&mut self, _kind: DocumentKind) {}
    fn save_document_manually(&mut self, _request: ManualSaveRequest) {}
    fn resolve_save_conflict(&mut self, _request: SaveConflictRequest) {}
    fn apply_product_settings_update(&mut self, _update: ProductSettingsUpdate) {}
    fn persist_layout(&mut self);
}

/// 将纯 reducer effect 交给唯一的外部操作边界。
pub struct EffectExecutor;

impl EffectExecutor {
    pub fn execute(service: &mut impl NotoraEffectService, effect: NotoraEffect) -> ShellEffect {
        match effect {
            NotoraEffect::QueryCards(query) => {
                service.query_cards(query);
                ShellEffect::NONE
            }
            NotoraEffect::ExecuteNoteCommand(command) => {
                service.execute_note_command(command);
                ShellEffect::NONE
            }
            NotoraEffect::ExecuteMetadataMutation(mutation) => {
                service.execute_metadata_mutation(mutation);
                ShellEffect::NONE
            }
            NotoraEffect::ExecuteTrashOperation(operation) => {
                service.execute_trash_operation(operation);
                ShellEffect::NONE
            }
            NotoraEffect::ChooseNoteRenameDestination(note_id) => {
                service.choose_note_rename_destination(note_id);
                ShellEffect::NONE
            }
            NotoraEffect::ChooseNoteMoveDirectory(note_id) => {
                service.choose_note_move_directory(note_id);
                ShellEffect::NONE
            }
            NotoraEffect::PrepareDocument(request) => {
                service.prepare_document(request);
                ShellEffect::NONE
            }
            NotoraEffect::PromoteActivePreview => {
                service.promote_active_preview();
                ShellEffect::NONE
            }
            NotoraEffect::OpenExternalFiles(request) => {
                service.open_external_files(request);
                ShellEffect::NONE
            }
            NotoraEffect::CreateUntitledExternal(kind) => {
                service.create_untitled_external(kind);
                ShellEffect::NONE
            }
            NotoraEffect::ResolveSaveConflict(request) => {
                service.resolve_save_conflict(request);
                ShellEffect::NONE
            }
            NotoraEffect::ApplyProductSettingsUpdate(update) => {
                service.apply_product_settings_update(update);
                ShellEffect::PERSIST_SETTINGS
            }
            NotoraEffect::PersistLayout => {
                service.persist_layout();
                ShellEffect::PERSIST_SETTINGS
            }
            NotoraEffect::Redraw => ShellEffect::REDRAW,
        }
    }

    /// 统一进入产品保存边界；reducer、窗口事件和 widget 不直接调用 runtime 保存 API。
    pub fn save_document_manually(
        service: &mut impl NotoraEffectService,
        request: ManualSaveRequest,
    ) {
        service.save_document_manually(request);
    }
}

#[cfg(test)]
mod tests {
    use super::{EffectExecutor, ManualSaveRequest, NotoraEffectService};
    use crate::action::{
        CardQuery, DocumentLoadRequest, MetadataMutation, NoteCreationTarget, NotoraEffect,
    };
    use notora_core::note_command::NoteCommand;
    use notora_core::{DocumentIdentity, DocumentKind, ExternalFileId, NavigationScope};

    #[derive(Default)]
    struct Recorder {
        card_query_count: usize,
        prepared_document: Option<DocumentLoadRequest>,
        executed_note_command_count: usize,
        metadata_mutation: Option<MetadataMutation>,
        promoted_preview_count: usize,
        manual_save_request: Option<ManualSaveRequest>,
    }

    impl NotoraEffectService for Recorder {
        fn query_cards(&mut self, _query: CardQuery) {
            self.card_query_count += 1;
        }

        fn request_note_creation(&mut self, _kind: DocumentKind, _target: NoteCreationTarget) {}

        fn execute_note_command(&mut self, _command: NoteCommand) {
            self.executed_note_command_count += 1;
        }

        fn execute_metadata_mutation(&mut self, mutation: MetadataMutation) {
            self.metadata_mutation = Some(mutation);
        }

        fn prepare_document(&mut self, request: DocumentLoadRequest) {
            self.prepared_document = Some(request);
        }

        fn promote_active_preview(&mut self) {
            self.promoted_preview_count += 1;
        }

        fn persist_layout(&mut self) {}

        fn save_document_manually(&mut self, request: ManualSaveRequest) {
            self.manual_save_request = Some(request);
        }
    }

    #[test]
    fn executor_routes_only_typed_effects_to_the_service_boundary() {
        let mut recorder = Recorder::default();
        assert_eq!(
            EffectExecutor::execute(
                &mut recorder,
                NotoraEffect::QueryCards(CardQuery::from(NavigationScope::Starred)),
            ),
            appkit_shell::ShellEffect::NONE
        );
        let identity = DocumentIdentity::ExternalFile(ExternalFileId::generate());
        let request = DocumentLoadRequest { identity, selection_generation: 3 };
        let _ = EffectExecutor::execute(&mut recorder, NotoraEffect::PrepareDocument(request));

        assert_eq!(recorder.card_query_count, 1);
        assert_eq!(recorder.prepared_document, Some(request));
        let _ = EffectExecutor::execute(&mut recorder, NotoraEffect::PromoteActivePreview);
        assert_eq!(recorder.promoted_preview_count, 1);
    }

    #[test]
    fn executor_routes_note_commands_without_exposing_file_io_to_the_reducer() {
        let mut recorder = Recorder::default();
        let command = notora_core::note_command::NoteCommand::Create(
            notora_core::note_command::CreateNoteRequest {
                kind: DocumentKind::Markdown,
                target_directory: None,
                tag_to_attach: None,
            },
        );

        assert_eq!(
            EffectExecutor::execute(&mut recorder, NotoraEffect::ExecuteNoteCommand(command)),
            appkit_shell::ShellEffect::NONE
        );
        assert_eq!(recorder.executed_note_command_count, 1);
    }

    #[test]
    fn executor_routes_metadata_mutations_through_the_typed_service_boundary() {
        let mut recorder = Recorder::default();
        let mutation = MetadataMutation::ToggleStar { note_id: notora_core::NoteId::generate() };

        assert_eq!(
            EffectExecutor::execute(
                &mut recorder,
                NotoraEffect::ExecuteMetadataMutation(mutation.clone())
            ),
            appkit_shell::ShellEffect::NONE
        );
        assert_eq!(recorder.metadata_mutation, Some(mutation));
    }

    #[test]
    fn manual_save_is_routed_through_the_effect_service_boundary() {
        let mut recorder = Recorder::default();
        let tab_id = appkit_core::workspace::types::TabIdAllocator::new().allocate();
        let request = ManualSaveRequest::ExistingExternalFile { tab_id };

        EffectExecutor::save_document_manually(&mut recorder, request);

        assert_eq!(recorder.manual_save_request, Some(request));
    }
}
