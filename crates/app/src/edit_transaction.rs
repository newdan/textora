use crate::document_view::DocumentView;
use crate::input::EditCommand;
use appkit_core::document::DocumentModel;
use appkit_core::edit::{TextEdit, apply_text_edit};
use ui::plugin::{
    EditIntent, EditPlan, EditRequest, EditSelection, EditTransaction, TextReplacement,
};

pub struct TransactionalEditOutcome {
    pub edit_outcome: crate::commands::EditOutcome,
    pub cursor_moved: bool,
    pub content_revision: u64,
    pub dirty: bool,
}

pub(crate) trait DocumentModelRef {
    fn document_model(&self) -> &DocumentModel;
}

pub(crate) trait DocumentModelMut: DocumentModelRef {
    fn document_model_mut(&mut self) -> &mut DocumentModel;
}

impl DocumentModelRef for DocumentModel {
    fn document_model(&self) -> &DocumentModel {
        self
    }
}

impl DocumentModelMut for DocumentModel {
    fn document_model_mut(&mut self) -> &mut DocumentModel {
        self
    }
}

impl DocumentModelRef for DocumentView {
    fn document_model(&self) -> &DocumentModel {
        &self.model
    }
}

impl DocumentModelMut for DocumentView {
    fn document_model_mut(&mut self) -> &mut DocumentModel {
        &mut self.model
    }
}

impl TransactionalEditOutcome {
    fn without_text_change(doc: &DocumentModel, cursor_moved: bool) -> Self {
        Self {
            edit_outcome: crate::commands::EditOutcome {
                executed: false,
                dirty_lines: None,
                old_line_count: doc.line_count(),
                new_line_count: doc.line_count(),
            },
            cursor_moved,
            content_revision: doc.content_revision(),
            dirty: doc.dirty,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum EditTransactionError {
    StaleGeneration { expected: u32, actual: u32 },
    OverlappingRanges { first_end: usize, second_start: usize },
    CursorOutOfBounds { cursor_after: usize, final_len: usize },
    InvalidRange { start: usize, end: usize, len: usize },
    InvalidCharBoundary { byte: usize },
    InvalidGraphemeBoundary { byte: usize },
    UnresolvedDefaultPlan,
}

fn map_core_edit_error(error: appkit_core::edit::EditError) -> EditTransactionError {
    match error {
        appkit_core::edit::EditError::StaleGeneration { expected, actual } => {
            EditTransactionError::StaleGeneration { expected, actual }
        }
        appkit_core::edit::EditError::InvalidRange { start, end, len } => {
            EditTransactionError::InvalidRange { start, end, len }
        }
        appkit_core::edit::EditError::InvalidCharBoundary { byte } => {
            EditTransactionError::InvalidCharBoundary { byte }
        }
        appkit_core::edit::EditError::InvalidGraphemeBoundary { byte } => {
            EditTransactionError::InvalidGraphemeBoundary { byte }
        }
    }
}

fn assign_dirty_snapshot_id_if_needed(doc: &mut DocumentModel) {
    if doc.dirty && doc.dirty_snapshot_id.is_none() {
        doc.dirty_snapshot_id = Some(if doc.file_path.is_some() {
            crate::dirty_snapshot::snapshot_filename(&crate::dirty_snapshot::path_id(
                doc.file_path.as_ref().expect("file path existence checked before snapshot id"),
            ))
        } else {
            crate::dirty_snapshot::snapshot_filename(&crate::dirty_snapshot::untitled_id())
        });
    }
}

pub fn edit_intent_for_command(command: &EditCommand) -> Option<EditIntent> {
    match command {
        EditCommand::InsertChar(text) | EditCommand::InsertText(text) => {
            Some(EditIntent::InsertText(text.clone()))
        }
        EditCommand::Paste => None, // Paste might need clipboard access first, so it might not be directly converted here yet. Wait, for now let's just leave it if it's not simple text, but if it is, maybe it is handled before this point. Actually, Paste doesn't carry string in EditCommand enum. Let's ignore it for now or return None.
        EditCommand::InsertNewline => Some(EditIntent::InsertParagraphBreak),
        EditCommand::Backspace => Some(EditIntent::DeleteBackward),
        EditCommand::DeleteForward => Some(EditIntent::DeleteForward),
        EditCommand::Tab => Some(EditIntent::Indent),
        _ => None,
    }
}

pub fn build_edit_request(doc: &impl DocumentModelRef, intent: EditIntent) -> EditRequest {
    let doc = doc.document_model();
    // 零宽选区（anchor == cursor）视为无选区，与 appkit-shell 侧行为对齐。
    let selection =
        doc.selection_range().filter(|(start, end)| start < end).map(|(start, end)| start..end);

    EditRequest {
        source_generation: doc.generation(),
        cursor_byte: doc.cursor_offset().to_usize(),
        selection,
        intent,
    }
}

pub fn default_edit_plan(request: &EditRequest, doc: &impl DocumentModelRef) -> EditPlan {
    let doc = doc.document_model();
    match &request.intent {
        EditIntent::InsertText(text) => replace_selection_or_cursor(request, text.clone()),
        EditIntent::InsertParagraphBreak => {
            replace_selection_or_cursor(request, default_newline_text(doc))
        }
        EditIntent::DeleteBackward => delete_selection_or_adjacent_grapheme(request, doc, -1),
        EditIntent::DeleteForward => delete_selection_or_adjacent_grapheme(request, doc, 1),
        EditIntent::Indent => replace_selection_or_cursor(request, default_indent_text(doc)),
        EditIntent::Outdent => default_outdent_plan(request, doc),
        EditIntent::PromoteObject | EditIntent::DemoteObject | EditIntent::SelectObject => {
            EditPlan::Consume
        }
    }
}

fn replace_selection_or_cursor(request: &EditRequest, text: String) -> EditPlan {
    let range = request.selection.clone().unwrap_or(request.cursor_byte..request.cursor_byte);
    let cursor_after = range.start + text.len();
    EditPlan::Apply(EditTransaction::replace(request.source_generation, range, text, cursor_after))
}

fn delete_selection_or_adjacent_grapheme(
    request: &EditRequest,
    doc: &DocumentModel,
    delta: isize,
) -> EditPlan {
    if let Some(range) = &request.selection {
        EditPlan::Apply(EditTransaction::replace(
            request.source_generation,
            range.clone(),
            String::new(),
            range.start,
        ))
    } else {
        use core::types::ByteIndex;
        let target =
            doc.tb.grapheme_boundary_delta(ByteIndex(request.cursor_byte), delta).to_usize();
        let (start, end) = if target < request.cursor_byte {
            (target, request.cursor_byte)
        } else {
            (request.cursor_byte, target)
        };
        EditPlan::Apply(EditTransaction::replace(
            request.source_generation,
            start..end,
            String::new(),
            start,
        ))
    }
}

fn default_indent_text(doc: &DocumentModel) -> String {
    if doc.tb.indent_with_tabs() { "\t".into() } else { " ".repeat(doc.tb.tab_size() as usize) }
}

fn default_newline_text(doc: &DocumentModel) -> String {
    if doc.tb.is_crlf() { "\r\n".into() } else { "\n".into() }
}

fn default_outdent_plan(_request: &EditRequest, _doc: &DocumentModel) -> EditPlan {
    // Basic outdent fallback
    EditPlan::Consume
}

pub fn validate_edit_transaction(
    source: &str,
    transaction: &EditTransaction,
) -> Result<(), EditTransactionError> {
    let replacements = sorted_replacements(transaction)?;
    for replacement in &replacements {
        validate_replacement_range(source, replacement)?;
    }

    let mut final_source = source.to_owned();
    for replacement in replacements.iter().rev() {
        final_source.replace_range(replacement.range.clone(), &replacement.text);
    }

    validate_selection(&final_source, &transaction.selection_after)
}

fn sorted_replacements(
    transaction: &EditTransaction,
) -> Result<Vec<&TextReplacement>, EditTransactionError> {
    let mut replacements: Vec<_> = transaction.replacements.iter().collect();
    replacements.sort_by_key(|replacement| replacement.range.start);
    for pair in replacements.windows(2) {
        if pair[0].range.end > pair[1].range.start {
            return Err(EditTransactionError::OverlappingRanges {
                first_end: pair[0].range.end,
                second_start: pair[1].range.start,
            });
        }
    }
    Ok(replacements)
}

fn validate_source_generation(
    transaction: &EditTransaction,
    doc: &DocumentModel,
) -> Result<(), EditTransactionError> {
    if transaction.source_generation == doc.generation() {
        return Ok(());
    }
    Err(EditTransactionError::StaleGeneration {
        expected: transaction.source_generation,
        actual: doc.generation(),
    })
}

fn validate_replacement_range(
    source: &str,
    replacement: &TextReplacement,
) -> Result<(), EditTransactionError> {
    let len = source.len();
    let range = &replacement.range;
    if range.start > range.end || range.end > len {
        return Err(EditTransactionError::InvalidRange { start: range.start, end: range.end, len });
    }

    if !source.is_char_boundary(range.start) {
        return Err(EditTransactionError::InvalidCharBoundary { byte: range.start });
    }

    if !source.is_char_boundary(range.end) {
        return Err(EditTransactionError::InvalidCharBoundary { byte: range.end });
    }

    if !is_grapheme_boundary_in_text(source, range.start) {
        return Err(EditTransactionError::InvalidGraphemeBoundary { byte: range.start });
    }

    if !is_grapheme_boundary_in_text(source, range.end) {
        return Err(EditTransactionError::InvalidGraphemeBoundary { byte: range.end });
    }

    Ok(())
}

fn validate_selection(source: &str, selection: &EditSelection) -> Result<(), EditTransactionError> {
    match selection {
        EditSelection::Caret(cursor_after) => validate_cursor_update(source, *cursor_after),
        EditSelection::Range { anchor, cursor } => {
            validate_cursor_update(source, *anchor)?;
            validate_cursor_update(source, *cursor)
        }
    }
}

fn is_grapheme_boundary_in_text(text: &str, byte: usize) -> bool {
    if byte > text.len() || !text.is_char_boundary(byte) {
        return false;
    }

    let document = text.as_bytes();
    core::unicode::CursorNav::new(&document).goto_byte(core::types::ByteIndex(byte)).offset
        == core::types::ByteIndex(byte)
}

fn validate_cursor_update(source: &str, cursor_after: usize) -> Result<(), EditTransactionError> {
    if cursor_after > source.len() {
        return Err(EditTransactionError::CursorOutOfBounds {
            cursor_after,
            final_len: source.len(),
        });
    }

    if !source.is_char_boundary(cursor_after) {
        return Err(EditTransactionError::InvalidCharBoundary { byte: cursor_after });
    }

    if !is_grapheme_boundary_in_text(source, cursor_after) {
        return Err(EditTransactionError::InvalidGraphemeBoundary { byte: cursor_after });
    }

    Ok(())
}

pub(crate) fn execute_text_replacement(
    replacement: &TextReplacement,
    cursor_after: usize,
    doc: &mut impl DocumentModelMut,
) -> bool {
    let transaction = EditTransaction::replace(
        doc.document_model().generation(),
        replacement.range.clone(),
        replacement.text.clone(),
        cursor_after,
    );
    execute_edit_plan(EditPlan::Apply(transaction), doc, &[])
        .map(|outcome| outcome.edit_outcome.executed)
        .unwrap_or(false)
}

pub fn execute_edit_plan(
    plan: EditPlan,
    doc: &mut impl DocumentModelMut,
    _advance_cache: &[ui::render_geom::AdvanceCacheEntry],
) -> Result<TransactionalEditOutcome, EditTransactionError> {
    use crate::commands::EditOutcome;

    let doc = doc.document_model_mut();
    let cursor_before = doc.cursor_offset().to_usize();

    match plan {
        EditPlan::UseDefault => Err(EditTransactionError::UnresolvedDefaultPlan),
        EditPlan::Consume => Ok(TransactionalEditOutcome::without_text_change(doc, false)),
        EditPlan::MoveCursor(update) => {
            validate_cursor_update(&doc.full_text(), update.cursor_after)?;
            doc.cursor_move_to_offset(update.cursor_after);
            Ok(TransactionalEditOutcome::without_text_change(
                doc,
                doc.cursor_offset().to_usize() != cursor_before,
            ))
        }
        EditPlan::SetSelection(selection) => {
            validate_selection(&doc.full_text(), &selection)?;
            apply_selection(doc, selection);
            Ok(TransactionalEditOutcome::without_text_change(
                doc,
                doc.cursor_offset().to_usize() != cursor_before,
            ))
        }
        EditPlan::Apply(transaction) => {
            let old_line_count = doc.line_count();
            validate_source_generation(&transaction, doc)?;
            validate_edit_transaction(doc.full_text().as_str(), &transaction)?;
            let replacements = sorted_replacements(&transaction)?;
            if replacements.is_empty() {
                apply_selection(doc, transaction.selection_after);
                return Ok(TransactionalEditOutcome::without_text_change(
                    doc,
                    doc.cursor_offset().to_usize() != cursor_before,
                ));
            }

            if replacements.len() == 1 {
                let replacement = replacements[0];
                let core_outcome = apply_text_edit(
                    doc,
                    TextEdit {
                        source_generation: transaction.source_generation,
                        range: replacement.range.clone(),
                        replacement: replacement.text.clone(),
                    },
                )
                .map_err(map_core_edit_error)?;

                assign_dirty_snapshot_id_if_needed(doc);
                apply_selection(doc, transaction.selection_after);

                let new_line_count = doc.line_count();
                return Ok(TransactionalEditOutcome {
                    edit_outcome: EditOutcome {
                        executed: core_outcome.executed,
                        dirty_lines: Some(
                            core_outcome.dirty_line_start
                                ..core_outcome.dirty_line_end.min(old_line_count),
                        ),
                        old_line_count,
                        new_line_count,
                    },
                    cursor_moved: doc.cursor_offset().to_usize() != cursor_before,
                    content_revision: doc.content_revision(),
                    dirty: doc.dirty,
                });
            }

            // Calculate minimum line to invalidate cache based on the change range
            let start = replacements
                .first()
                .expect("non-empty replacements were checked before dirty-line calculation")
                .range
                .start;
            let end = replacements
                .last()
                .expect("non-empty replacements were checked before dirty-line calculation")
                .range
                .end;
            let min_line = match doc.line_index.offsets.binary_search(&start) {
                Ok(i) => i,
                Err(i) => i.saturating_sub(1),
            };
            let max_line = match doc.line_index.offsets.binary_search(&end) {
                Ok(i) => i,
                Err(i) => i.saturating_sub(1),
            };

            doc.tb.edit_begin_grouping();
            for replacement in replacements.iter().rev() {
                doc.tb.replace_range(replacement.range.clone(), replacement.text.as_bytes());
            }
            doc.tb.edit_end_grouping();
            doc.line_index = appkit_core::line_index::LineIndex::rebuild_from(&doc.tb);
            doc.mark_content_changed();
            doc.dirty = doc.tb.is_dirty();
            doc.sync_cursor_from_buffer();
            apply_selection(doc, transaction.selection_after);

            let new_line_count = doc.line_count();

            let lines_deleted = old_line_count.saturating_sub(new_line_count);
            let dirty_end = max_line + 1 + lines_deleted;
            let dirty_lines = Some(min_line..dirty_end.min(old_line_count));
            Ok(TransactionalEditOutcome {
                edit_outcome: EditOutcome {
                    executed: true,
                    dirty_lines,
                    old_line_count,
                    new_line_count,
                },
                cursor_moved: doc.cursor_offset().to_usize() != cursor_before,
                content_revision: doc.content_revision(),
                dirty: doc.dirty,
            })
        }
    }
}

fn apply_selection(doc: &mut DocumentModel, selection: EditSelection) {
    match selection {
        EditSelection::Caret(byte) => {
            doc.cursor_move_to_offset(byte);
            doc.cursor_mut().selection_anchor = None;
        }
        EditSelection::Range { anchor, cursor } => {
            doc.cursor_move_to_offset(cursor);
            doc.cursor_mut().selection_anchor = Some(anchor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;
    use ui::plugin::{CursorUpdate, EditSelection};

    fn document_from_text(text: &str) -> DocumentModel {
        let mut text_buffer =
            core::buffer::TextBuffer::new(false).expect("TextBuffer creation should succeed");
        text_buffer.write_raw(text.as_bytes());
        text_buffer.mark_as_clean();
        DocumentModel::new(text_buffer)
    }

    fn request_at(cursor_byte: usize, intent: EditIntent) -> EditRequest {
        EditRequest { source_generation: 1, cursor_byte, selection: None, intent }
    }

    fn request_with_selection(selection: Range<usize>, intent: EditIntent) -> EditRequest {
        EditRequest {
            source_generation: 1,
            cursor_byte: selection.end,
            selection: Some(selection),
            intent,
        }
    }

    fn apply(
        source_generation: u32,
        range: Range<usize>,
        text: &str,
        cursor_after: usize,
    ) -> EditPlan {
        EditPlan::Apply(EditTransaction::replace(
            source_generation,
            range,
            text.to_owned(),
            cursor_after,
        ))
    }

    #[test]
    fn execute_multiple_replacements_is_atomic_and_undoes_once() {
        let mut doc = document_from_text("# Root\n## Child\n### Leaf\n");
        let generation = doc.generation();
        let plan = EditPlan::Apply(EditTransaction {
            source_generation: generation,
            replacements: vec![
                TextReplacement { range: 7..7, text: "#".into() },
                TextReplacement { range: 16..16, text: "#".into() },
            ],
            selection_after: EditSelection::Caret(20),
        });

        execute_edit_plan(plan, &mut doc, &[]).expect("valid grouped transaction");
        assert_eq!(doc.full_text(), "# Root\n### Child\n#### Leaf\n");

        doc.undo();
        assert_eq!(doc.full_text(), "# Root\n## Child\n### Leaf\n");
    }

    #[test]
    fn overlapping_replacements_are_rejected_without_writing() {
        let mut doc = document_from_text("abcdef");
        let generation = doc.generation();
        let plan = EditPlan::Apply(EditTransaction {
            source_generation: generation,
            replacements: vec![
                TextReplacement { range: 1..4, text: "X".into() },
                TextReplacement { range: 3..5, text: "Y".into() },
            ],
            selection_after: EditSelection::Caret(2),
        });

        assert!(matches!(
            execute_edit_plan(plan, &mut doc, &[]),
            Err(EditTransactionError::OverlappingRanges { first_end: 4, second_start: 3 })
        ));
        assert_eq!(doc.full_text(), "abcdef");
    }

    #[test]
    fn stale_generation_is_rejected_without_writing() {
        let mut doc = document_from_text("abc");
        let stale_generation = doc.generation().wrapping_sub(1);
        let plan = EditPlan::Apply(EditTransaction::replace(stale_generation, 1..2, "Z".into(), 2));

        assert!(matches!(
            execute_edit_plan(plan, &mut doc, &[]),
            Err(EditTransactionError::StaleGeneration { .. })
        ));
        assert_eq!(doc.full_text(), "abc");
    }

    #[test]
    fn set_selection_changes_focus_without_creating_text_edit() {
        let mut doc = document_from_text("abcdef");
        let plan = EditPlan::SetSelection(EditSelection::Range { anchor: 1, cursor: 5 });
        let outcome = execute_edit_plan(plan, &mut doc, &[]).expect("valid selection update");
        assert_eq!(doc.selection_range(), Some((1, 5)));
        assert!(!outcome.edit_outcome.executed);
    }

    #[test]
    fn default_backspace_deletes_one_grapheme_and_keeps_cursor_at_range_start() {
        let emoji = "👨\u{200D}👩\u{200D}👧";
        let mut doc = document_from_text(&format!("a{emoji}b"));
        let emoji_end = 1 + emoji.len();
        doc.cursor_move_to_offset(emoji_end);
        let request = build_edit_request(&doc, EditIntent::DeleteBackward);

        let plan = default_edit_plan(&request, &doc);

        assert_eq!(
            plan,
            EditPlan::Apply(EditTransaction::replace(
                request.source_generation,
                1..emoji_end,
                String::new(),
                1,
            ))
        );
    }

    #[test]
    fn default_delete_with_selection_only_deletes_selection() {
        let mut doc = document_from_text("abcdef");
        doc.cursor_move_to_offset(5);
        doc.cursor_mut().selection_anchor = Some(2);
        let request = build_edit_request(&doc, EditIntent::DeleteForward);

        assert_eq!(
            default_edit_plan(&request, &doc),
            EditPlan::Apply(EditTransaction::replace(
                request.source_generation,
                2..5,
                String::new(),
                2,
            ))
        );
    }

    #[test]
    fn default_paragraph_break_in_crlf_document_uses_crlf_sequence() {
        let mut doc = document_from_text("firstsecond");
        doc.tb.set_crlf(true);
        doc.cursor_move_to_offset("first".len());
        let request = build_edit_request(&doc, EditIntent::InsertParagraphBreak);

        let plan = default_edit_plan(&request, &doc);
        execute_edit_plan(plan, &mut doc, &[]).expect("CRLF paragraph break should be valid");

        assert_eq!(doc.full_text(), "first\r\nsecond");
        assert_eq!(doc.cursor_offset().to_usize(), "first\r\n".len());
    }

    #[test]
    fn default_structural_intents_are_consumed_without_writing_source() {
        let mut doc = document_from_text("# Root");
        let source_before = doc.full_text();
        let generation_before = doc.generation();

        for intent in
            [EditIntent::PromoteObject, EditIntent::DemoteObject, EditIntent::SelectObject]
        {
            let request = build_edit_request(&doc, intent);
            let plan = default_edit_plan(&request, &doc);
            assert_eq!(plan, EditPlan::Consume);

            let outcome = execute_edit_plan(plan, &mut doc, &[])
                .expect("consuming structural intent is valid");
            assert!(!outcome.edit_outcome.executed);
            assert_eq!(doc.full_text(), source_before);
            assert_eq!(doc.generation(), generation_before);
        }
    }

    #[test]
    fn validator_rejects_cursor_after_final_text() {
        let transaction = EditTransaction::replace(0, 1..2, String::new(), 9);

        assert_eq!(
            validate_edit_transaction("abc", &transaction),
            Err(EditTransactionError::CursorOutOfBounds { cursor_after: 9, final_len: 2 })
        );
    }

    #[test]
    fn execute_apply_replaces_selection_as_one_edit_and_clears_anchor() {
        let mut doc = document_from_text("hello world");
        let generation_before = doc.generation();
        let plan = EditPlan::Apply(EditTransaction::replace(
            generation_before,
            5..11,
            "\n\nnext".into(),
            7,
        ));

        let outcome = execute_edit_plan(plan, &mut doc, &[]).expect("valid transaction");

        assert_eq!(doc.full_text(), "hello\n\nnext");
        assert_eq!(doc.cursor_offset().to_usize(), 7);
        assert!(doc.cursor().selection_anchor.is_none());
        assert!(doc.generation() > generation_before);
        assert!(outcome.edit_outcome.executed);
        assert_eq!(outcome.content_revision, doc.content_revision());
        assert!(outcome.dirty);
    }

    #[test]
    fn execute_nonempty_replacement_increments_generation_once_and_undoes_once() {
        let mut doc = document_from_text("hello world");
        let generation_before = doc.generation();
        let plan = apply(generation_before, 5..11, "next", 9);

        execute_edit_plan(plan, &mut doc, &[]).expect("transaction must be valid");

        assert_eq!(doc.full_text(), "hellonext");
        assert_eq!(doc.generation(), generation_before + 1);
        doc.undo();
        assert_eq!(doc.full_text(), "hello world");
    }

    #[test]
    fn validator_rejects_range_inside_grapheme_cluster() {
        let source = "e\u{301}x";
        assert!(
            validate_edit_transaction(source, &EditTransaction::replace(0, 1..3, "Q".into(), 2),)
                .is_err()
        );
    }

    #[test]
    fn validator_rejects_cursor_inside_inserted_utf8_character() {
        assert!(
            validate_edit_transaction("", &EditTransaction::replace(0, 0..0, "中".into(), 1),)
                .is_err()
        );
    }

    #[test]
    fn validator_rejects_cursor_inside_inserted_combining_grapheme() {
        assert!(
            validate_edit_transaction(
                "",
                &EditTransaction::replace(0, 0..0, "e\u{301}".into(), 1),
            )
            .is_err()
        );
    }

    #[test]
    fn direct_replacement_rejects_cursor_inside_inserted_combining_grapheme() {
        let mut doc = document_from_text("");
        let replacement = TextReplacement { range: 0..0, text: "e\u{301}".into() };

        assert!(!execute_text_replacement(&replacement, 1, &mut doc));
        assert_eq!(doc.full_text(), "");
    }

    #[test]
    fn move_cursor_rejects_out_of_bounds_and_non_grapheme_positions() {
        let mut doc = document_from_text("e\u{301}x");
        assert!(
            execute_edit_plan(
                EditPlan::MoveCursor(CursorUpdate { cursor_after: 99 }),
                &mut doc,
                &[],
            )
            .is_err()
        );
        assert!(
            execute_edit_plan(
                EditPlan::MoveCursor(CursorUpdate { cursor_after: 1 }),
                &mut doc,
                &[],
            )
            .is_err()
        );
    }

    #[test]
    fn execute_move_cursor_does_not_change_generation() {
        let mut doc = document_from_text("abc");
        let generation_before = doc.generation();

        let outcome = execute_edit_plan(
            EditPlan::MoveCursor(CursorUpdate { cursor_after: 2 }),
            &mut doc,
            &[],
        )
        .expect("cursor update is valid");

        assert_eq!(doc.generation(), generation_before);
        assert_eq!(doc.cursor_offset().to_usize(), 2);
        assert!(!outcome.edit_outcome.executed);
        assert!(outcome.cursor_moved);
    }

    #[test]
    fn build_edit_request_filters_zero_width_selection() {
        let mut doc = document_from_text("abc");
        doc.cursor_move_to_offset(2);
        doc.cursor_mut().selection_anchor = Some(2);

        let request = build_edit_request(&doc, EditIntent::InsertParagraphBreak);

        assert_eq!(request.selection, None, "零宽选区必须视为无选区");
    }

    #[test]
    fn build_edit_request_keeps_non_empty_selection() {
        let mut doc = document_from_text("abc");
        doc.cursor_move_to_offset(3);
        doc.cursor_mut().selection_anchor = Some(1);

        let request = build_edit_request(&doc, EditIntent::InsertParagraphBreak);

        assert_eq!(request.selection, Some(1..3));
    }

    #[test]
    fn delete_forward_and_tab_are_transactional_edit_intents() {
        assert_eq!(
            edit_intent_for_command(&EditCommand::DeleteForward),
            Some(EditIntent::DeleteForward)
        );
        assert_eq!(edit_intent_for_command(&EditCommand::Tab), Some(EditIntent::Indent));
    }
}
