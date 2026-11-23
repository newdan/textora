#[cfg(test)]
use crate::document_view::DocumentView;
use appkit_core::document::DocumentModel;

pub(crate) use appkit_shell::edit_transaction::edit_intent_for_command;

#[cfg(test)]
pub(crate) use appkit_shell::edit_transaction::{
    TransactionalEditOutcome, build_edit_request, default_edit_plan, execute_edit_plan,
};

pub(crate) trait DocumentModelRef {
    fn document_model(&self) -> &DocumentModel;
}

#[cfg(test)]
pub(crate) trait DocumentModelMut: DocumentModelRef {
    fn document_model_mut(&mut self) -> &mut DocumentModel;
}

impl DocumentModelRef for DocumentModel {
    fn document_model(&self) -> &DocumentModel {
        self
    }
}

#[cfg(test)]
impl DocumentModelMut for DocumentModel {
    fn document_model_mut(&mut self) -> &mut DocumentModel {
        self
    }
}

#[cfg(test)]
impl DocumentModelRef for DocumentView {
    fn document_model(&self) -> &DocumentModel {
        &self.model
    }
}

#[cfg(test)]
impl DocumentModelMut for DocumentView {
    fn document_model_mut(&mut self) -> &mut DocumentModel {
        &mut self.model
    }
}
