//! notora effect executor boundary.

use appkit_shell::ShellEffect;
use std::path::PathBuf;

use crate::action::{CardQuery, DocumentLoadRequest, NoteCreationTarget, NotoraEffect};
use notora_core::DocumentKind;
use notora_core::note_command::NoteCommand;

/// 外部打开来源最终统一为同一个 effect；路径来源不应拥有单独的验证逻辑。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalOpenRequest {
    ShowFileDialog,
    Paths(Vec<PathBuf>),
}

/// 产品层外部能力的唯一入口。实现者可调度 worker、dialog、catalog 或 runtime。
pub trait NotoraEffectService {
    fn query_cards(&mut self, query: CardQuery);
    fn request_note_creation(&mut self, kind: DocumentKind, target: NoteCreationTarget);
    fn execute_note_command(&mut self, _command: NoteCommand) {}
    fn prepare_document(&mut self, request: DocumentLoadRequest);
    fn promote_active_preview(&mut self) {}
    fn open_external_files(&mut self, _request: ExternalOpenRequest) {}
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
            NotoraEffect::PersistLayout => {
                service.persist_layout();
                ShellEffect::PERSIST_SETTINGS
            }
            NotoraEffect::Redraw => ShellEffect::REDRAW,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EffectExecutor, NotoraEffectService};
    use crate::action::{CardQuery, DocumentLoadRequest, NoteCreationTarget, NotoraEffect};
    use notora_core::note_command::NoteCommand;
    use notora_core::{DocumentIdentity, DocumentKind, ExternalFileId, NavigationScope};

    #[derive(Default)]
    struct Recorder {
        card_query_count: usize,
        prepared_document: Option<DocumentLoadRequest>,
        executed_note_command_count: usize,
        promoted_preview_count: usize,
    }

    impl NotoraEffectService for Recorder {
        fn query_cards(&mut self, _query: CardQuery) {
            self.card_query_count += 1;
        }

        fn request_note_creation(&mut self, _kind: DocumentKind, _target: NoteCreationTarget) {}

        fn execute_note_command(&mut self, _command: NoteCommand) {
            self.executed_note_command_count += 1;
        }

        fn prepare_document(&mut self, request: DocumentLoadRequest) {
            self.prepared_document = Some(request);
        }

        fn promote_active_preview(&mut self) {
            self.promoted_preview_count += 1;
        }

        fn persist_layout(&mut self) {}
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
}
