//! notora effect executor boundary.

use appkit_shell::ShellEffect;

use crate::action::{CardQuery, NoteCreationTarget, NotoraEffect};
use notora_core::{DocumentIdentity, DocumentKind};

/// 产品层外部能力的唯一入口。实现者可调度 worker、dialog、catalog 或 runtime。
pub trait NotoraEffectService {
    fn query_cards(&mut self, query: CardQuery);
    fn request_note_creation(&mut self, kind: DocumentKind, target: NoteCreationTarget);
    fn prepare_document(&mut self, identity: DocumentIdentity);
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
            NotoraEffect::RequestNoteCreation { kind, target } => {
                service.request_note_creation(kind, target);
                ShellEffect::NONE
            }
            NotoraEffect::PrepareDocument(identity) => {
                service.prepare_document(identity);
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
    use crate::action::{CardQuery, NoteCreationTarget, NotoraEffect};
    use notora_core::{DocumentIdentity, DocumentKind, ExternalFileId, NavigationScope};

    #[derive(Default)]
    struct Recorder {
        card_query_count: usize,
        prepared_document: Option<DocumentIdentity>,
    }

    impl NotoraEffectService for Recorder {
        fn query_cards(&mut self, _query: CardQuery) {
            self.card_query_count += 1;
        }

        fn request_note_creation(&mut self, _kind: DocumentKind, _target: NoteCreationTarget) {}

        fn prepare_document(&mut self, identity: DocumentIdentity) {
            self.prepared_document = Some(identity);
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
        let _ = EffectExecutor::execute(&mut recorder, NotoraEffect::PrepareDocument(identity));

        assert_eq!(recorder.card_query_count, 1);
        assert_eq!(recorder.prepared_document, Some(identity));
    }
}
