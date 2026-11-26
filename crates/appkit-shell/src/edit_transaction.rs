use appkit_core::document::DocumentModel;
use appkit_core::edit::{TextEdit, apply_text_edit};
use appkit_core::edit_command::EditCommand;
use textora_core::buffer::EditHistoryKind;
use textora_core::unicode::is_grapheme_boundary_in_text;
use ui::plugin::{
    EditIntent, EditPlan, EditRequest, EditSelection, EditTransaction, TextReplacement,
};

/// 编辑命令的副作用范围。
#[derive(Default, Debug, Clone)]
pub struct EditOutcome {
    /// 命令是否真实执行。
    pub executed: bool,
    /// 受影响的 doc line 区间 [start, end_exclusive)。
    pub dirty_lines: Option<std::ops::Range<usize>>,
    /// 命令执行前的 line_count。
    pub old_line_count: usize,
    /// 命令执行后的 line_count。
    pub new_line_count: usize,
}

impl EditOutcome {
    pub fn invalidates_all_render_cache(&self) -> bool {
        self.dirty_lines.is_some() && self.new_line_count != self.old_line_count
    }

    pub fn render_cache_invalidation_range(&self) -> Option<std::ops::Range<usize>> {
        if self.invalidates_all_render_cache() {
            return None;
        }
        let dirty_lines = self.dirty_lines.as_ref()?;
        Some(dirty_lines.clone())
    }
}

pub struct TransactionalEditOutcome {
    pub edit_outcome: EditOutcome,
    pub cursor_moved: bool,
    pub content_revision: u64,
    pub dirty: bool,
}

impl TransactionalEditOutcome {
    fn without_text_change(doc: &DocumentModel, cursor_moved: bool) -> Self {
        Self {
            edit_outcome: EditOutcome {
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
    use appkit_core::snapshot::{path_id, snapshot_filename, untitled_id};

    if !doc.dirty || doc.dirty_snapshot_id.is_some() {
        return;
    }
    let snapshot_id = doc.file_path.as_deref().map_or_else(untitled_id, path_id);
    doc.dirty_snapshot_id = Some(snapshot_filename(&snapshot_id));
}

pub fn edit_intent_for_command(command: &EditCommand) -> Option<EditIntent> {
    match command {
        EditCommand::InsertChar(text) | EditCommand::InsertText(text) => {
            Some(EditIntent::InsertText(text.clone()))
        }
        EditCommand::InsertNewline => Some(EditIntent::InsertParagraphBreak),
        EditCommand::Backspace => Some(EditIntent::DeleteBackward),
        EditCommand::DeleteForward => Some(EditIntent::DeleteForward),
        EditCommand::Tab => Some(EditIntent::Indent),
        _ => None,
    }
}

pub fn build_edit_request(doc: &DocumentModel, intent: EditIntent) -> EditRequest {
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

pub fn default_edit_plan(request: &EditRequest, doc: &DocumentModel) -> EditPlan {
    let plan = match &request.intent {
        EditIntent::InsertText(text) => replace_selection_or_cursor(request, text.clone()),
        EditIntent::InsertParagraphBreak | EditIntent::InsertLineBreak => {
            replace_selection_or_cursor(request, default_newline_text(doc))
        }
        EditIntent::DeleteBackward => delete_selection_or_adjacent_grapheme(request, doc, -1),
        EditIntent::DeleteForward => delete_selection_or_adjacent_grapheme(request, doc, 1),
        EditIntent::Indent => replace_selection_or_cursor(request, default_indent_text(doc)),
        EditIntent::Outdent
        | EditIntent::PromoteObject
        | EditIntent::DemoteObject
        | EditIntent::SelectObject => EditPlan::Consume,
    };
    tag_default_plan(plan, &request.intent)
}

/// Marks a resolved default plan with its undo coalescing kind, so the
/// execution layer can tell it apart from plugin-augmented [`EditPlan::Apply`]
/// transactions (which must always stay independent undo entries).
fn tag_default_plan(plan: EditPlan, intent: &EditIntent) -> EditPlan {
    let EditPlan::Apply(transaction) = plan else {
        return plan;
    };
    EditPlan::ApplyDefault(transaction, default_history_kind(intent))
}

fn default_history_kind(intent: &EditIntent) -> EditHistoryKind {
    match intent {
        EditIntent::InsertText(_) => EditHistoryKind::Insert,
        EditIntent::DeleteBackward | EditIntent::DeleteForward => EditHistoryKind::Delete,
        _ => EditHistoryKind::Standalone,
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
        use textora_core::types::ByteIndex;
        let target =
            doc.tb.grapheme_boundary_delta(ByteIndex(request.cursor_byte), delta).to_usize();
        let (start, end) = if target < request.cursor_byte {
            (target, request.cursor_byte)
        } else {
            (request.cursor_byte, target)
        };
        if start == end {
            return EditPlan::Consume;
        }
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

pub fn validate_edit_transaction(
    source: &str,
    transaction: &EditTransaction,
) -> Result<(), EditTransactionError> {
    let replacements = sorted_replacements(transaction)?;
    validated_final_source(source, &replacements, &transaction.selection_after).map(|_| ())
}

fn validated_final_source(
    source: &str,
    replacements: &[&TextReplacement],
    selection_after: &EditSelection,
) -> Result<String, EditTransactionError> {
    for replacement in replacements {
        validate_replacement_range(source, replacement)?;
    }

    let mut final_source = source.to_owned();
    for replacement in replacements.iter().rev() {
        final_source.replace_range(replacement.range.clone(), &replacement.text);
    }
    validate_selection(&final_source, selection_after)?;
    Ok(final_source)
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

pub fn execute_text_replacement(
    replacement: &TextReplacement,
    cursor_after: usize,
    doc: &mut DocumentModel,
) -> bool {
    let transaction = EditTransaction::replace(
        doc.generation(),
        replacement.range.clone(),
        replacement.text.clone(),
        cursor_after,
    );
    execute_edit_plan(EditPlan::Apply(transaction), doc)
        .map(|outcome| outcome.edit_outcome.executed)
        .unwrap_or(false)
}

pub fn execute_edit_plan(
    plan: EditPlan,
    doc: &mut DocumentModel,
) -> Result<TransactionalEditOutcome, EditTransactionError> {
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
            execute_apply_transaction(doc, transaction, EditHistoryKind::Standalone, cursor_before)
        }
        EditPlan::ApplyDefault(transaction, history) => {
            execute_apply_transaction(doc, transaction, history, cursor_before)
        }
    }
}

fn execute_apply_transaction(
    doc: &mut DocumentModel,
    transaction: EditTransaction,
    history: EditHistoryKind,
    cursor_before: usize,
) -> Result<TransactionalEditOutcome, EditTransactionError> {
    let old_line_count = doc.line_count();
    validate_source_generation(&transaction, doc)?;
    let replacements = sorted_replacements(&transaction)?;
    let source = doc.full_text();
    let Some((first, last)) = replacements.first().zip(replacements.last()) else {
        validate_selection(&source, &transaction.selection_after)?;
        apply_selection(doc, transaction.selection_after);
        return Ok(TransactionalEditOutcome::without_text_change(
            doc,
            doc.cursor_offset().to_usize() != cursor_before,
        ));
    };

    let final_source =
        validated_final_source(&source, &replacements, &transaction.selection_after)?;
    // Commit the edited span once, so one replacement cannot alter the grapheme
    // boundaries used to locate another replacement in the original document.
    let range = first.range.start..last.range.end;
    let replacement_end = final_source.len() - (source.len() - range.end);
    let replacement = final_source[range.start..replacement_end].to_owned();
    let history = if replacements.len() == 1 { history } else { EditHistoryKind::Standalone };
    let core_outcome = apply_text_edit(
        doc,
        TextEdit { source_generation: transaction.source_generation, range, replacement, history },
    )
    .map_err(map_core_edit_error)?;

    assign_dirty_snapshot_id_if_needed(doc);
    apply_selection(doc, transaction.selection_after);

    let new_line_count = doc.line_count();
    Ok(TransactionalEditOutcome {
        edit_outcome: EditOutcome {
            executed: core_outcome.executed,
            dirty_lines: Some(
                core_outcome.dirty_line_start..core_outcome.dirty_line_end.min(old_line_count),
            ),
            old_line_count,
            new_line_count,
        },
        cursor_moved: doc.cursor_offset().to_usize() != cursor_before,
        content_revision: doc.content_revision(),
        dirty: doc.dirty,
    })
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
        let mut text_buffer = textora_core::buffer::TextBuffer::new(false)
            .expect("TextBuffer creation should succeed");
        text_buffer.write_raw(text.as_bytes());
        text_buffer.mark_as_clean();
        DocumentModel::new(text_buffer)
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

        execute_edit_plan(plan, &mut doc).expect("valid grouped transaction");
        assert_eq!(doc.full_text(), "# Root\n### Child\n#### Leaf\n");

        doc.undo();
        assert_eq!(doc.full_text(), "# Root\n## Child\n### Leaf\n");
    }

    #[test]
    fn grouped_replacements_keep_source_offsets_when_clusters_join() {
        let mut doc = document_from_text("ab");
        let plan = EditPlan::Apply(EditTransaction {
            source_generation: doc.generation(),
            replacements: vec![
                TextReplacement { range: 0..1, text: "x".into() },
                TextReplacement { range: 1..2, text: "\u{301}".into() },
            ],
            selection_after: EditSelection::Caret(3),
        });

        execute_edit_plan(plan, &mut doc).expect("the edit has valid source and final boundaries");

        assert_eq!(doc.full_text(), "x\u{301}");
        assert_eq!(doc.cursor_offset().to_usize(), 3);
        doc.undo();
        assert_eq!(doc.full_text(), "ab");
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
            execute_edit_plan(plan, &mut doc),
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
            execute_edit_plan(plan, &mut doc),
            Err(EditTransactionError::StaleGeneration { .. })
        ));
        assert_eq!(doc.full_text(), "abc");
    }

    #[test]
    fn set_selection_changes_focus_without_creating_text_edit() {
        let mut doc = document_from_text("abcdef");
        let plan = EditPlan::SetSelection(EditSelection::Range { anchor: 1, cursor: 5 });
        let outcome = execute_edit_plan(plan, &mut doc).expect("valid selection update");
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
            EditPlan::ApplyDefault(
                EditTransaction::replace(request.source_generation, 1..emoji_end, String::new(), 1,),
                EditHistoryKind::Delete,
            )
        );
    }

    #[test]
    fn default_delete_at_document_boundary_is_consumed_without_transaction() {
        let mut doc = document_from_text("abc");
        doc.cursor_move_to_offset(0);
        let backward_request = build_edit_request(&doc, EditIntent::DeleteBackward);
        assert_eq!(default_edit_plan(&backward_request, &doc), EditPlan::Consume);

        doc.cursor_move_to_offset(doc.full_text().len());
        let forward_request = build_edit_request(&doc, EditIntent::DeleteForward);
        assert_eq!(default_edit_plan(&forward_request, &doc), EditPlan::Consume);
    }

    #[test]
    fn default_delete_with_selection_only_deletes_selection() {
        let mut doc = document_from_text("abcdef");
        doc.cursor_move_to_offset(5);
        doc.cursor_mut().selection_anchor = Some(2);
        let request = build_edit_request(&doc, EditIntent::DeleteForward);

        assert_eq!(
            default_edit_plan(&request, &doc),
            EditPlan::ApplyDefault(
                EditTransaction::replace(request.source_generation, 2..5, String::new(), 2,),
                EditHistoryKind::Delete,
            )
        );
    }

    #[test]
    fn default_paragraph_break_in_crlf_document_uses_crlf_sequence() {
        let mut doc = document_from_text("firstsecond");
        doc.tb.set_crlf(true);
        doc.cursor_move_to_offset("first".len());
        let request = build_edit_request(&doc, EditIntent::InsertParagraphBreak);

        let plan = default_edit_plan(&request, &doc);
        execute_edit_plan(plan, &mut doc).expect("CRLF paragraph break should be valid");

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

            let outcome =
                execute_edit_plan(plan, &mut doc).expect("consuming structural intent is valid");
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

        let outcome = execute_edit_plan(plan, &mut doc).expect("valid transaction");

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

        execute_edit_plan(plan, &mut doc).expect("transaction must be valid");

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
            execute_edit_plan(EditPlan::MoveCursor(CursorUpdate { cursor_after: 99 }), &mut doc,)
                .is_err()
        );
        assert!(
            execute_edit_plan(EditPlan::MoveCursor(CursorUpdate { cursor_after: 1 }), &mut doc,)
                .is_err()
        );
    }

    #[test]
    fn execute_move_cursor_does_not_change_generation() {
        let mut doc = document_from_text("abc");
        let generation_before = doc.generation();

        let outcome =
            execute_edit_plan(EditPlan::MoveCursor(CursorUpdate { cursor_after: 2 }), &mut doc)
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

    fn execute_default_plan(doc: &mut DocumentModel, intent: EditIntent) {
        let request = build_edit_request(&*doc, intent);
        let plan = default_edit_plan(&request, &*doc);
        execute_edit_plan(plan, doc).expect("default plan must execute");
    }

    #[test]
    fn default_insert_text_plans_coalesce_into_single_undo_entry() {
        let mut doc = document_from_text("");

        for text in ["a", "b", "c"] {
            execute_default_plan(&mut doc, EditIntent::InsertText(text.into()));
        }

        assert_eq!(doc.full_text(), "abc");
        doc.undo();
        assert_eq!(doc.full_text(), "", "one undo must revert the whole typing run");
    }

    #[test]
    fn default_backspace_plans_coalesce_into_single_undo_entry() {
        let mut doc = document_from_text("abc");
        doc.cursor_move_to_offset(3);

        for _ in 0..3 {
            execute_default_plan(&mut doc, EditIntent::DeleteBackward);
        }

        assert_eq!(doc.full_text(), "");
        doc.undo();
        assert_eq!(doc.full_text(), "abc", "one undo must revert the whole backspace run");
    }

    #[test]
    fn default_paragraph_break_does_not_coalesce_with_surrounding_typing() {
        let mut doc = document_from_text("");

        execute_default_plan(&mut doc, EditIntent::InsertText("a".into()));
        execute_default_plan(&mut doc, EditIntent::InsertParagraphBreak);
        execute_default_plan(&mut doc, EditIntent::InsertText("b".into()));

        assert_eq!(doc.full_text(), "a\nb");
        doc.undo();
        assert_eq!(doc.full_text(), "a\n", "first undo must only remove the typed character");
        doc.undo();
        assert_eq!(doc.full_text(), "a", "second undo must only remove the paragraph break");
        doc.undo();
        assert_eq!(doc.full_text(), "");
    }

    #[test]
    fn default_indent_does_not_coalesce_with_following_typing() {
        let mut doc = document_from_text("");

        execute_default_plan(&mut doc, EditIntent::Indent);
        let indented = doc.full_text();
        execute_default_plan(&mut doc, EditIntent::InsertText("x".into()));

        assert_eq!(doc.full_text(), format!("{indented}x"));
        doc.undo();
        assert_eq!(doc.full_text(), indented, "first undo must only remove the typed text");
        doc.undo();
        assert_eq!(doc.full_text(), "", "second undo must only remove the indent");
    }

    #[test]
    fn alternating_default_insert_and_delete_plans_split_undo_entries() {
        let mut doc = document_from_text("");

        execute_default_plan(&mut doc, EditIntent::InsertText("a".into()));
        execute_default_plan(&mut doc, EditIntent::DeleteBackward);
        execute_default_plan(&mut doc, EditIntent::InsertText("b".into()));

        assert_eq!(doc.full_text(), "b");
        doc.undo();
        assert_eq!(doc.full_text(), "");
        doc.undo();
        assert_eq!(doc.full_text(), "a");
        doc.undo();
        assert_eq!(doc.full_text(), "");
    }

    #[test]
    fn plugin_augmented_plan_stays_an_independent_undo_entry_between_typing_runs() {
        let mut doc = document_from_text("");

        execute_default_plan(&mut doc, EditIntent::InsertText("a".into()));
        execute_default_plan(&mut doc, EditIntent::InsertText("b".into()));
        // Plugin augmentation (e.g. smart punctuation) rewrites the just-typed text.
        let augmented =
            EditPlan::Apply(EditTransaction::replace(doc.generation(), 1..2, "*".into(), 2));
        execute_edit_plan(augmented, &mut doc).expect("augmented plan must execute");
        execute_default_plan(&mut doc, EditIntent::InsertText("c".into()));

        assert_eq!(doc.full_text(), "a*c");
        doc.undo();
        assert_eq!(doc.full_text(), "a*", "typing after augmentation must be its own entry");
        doc.undo();
        assert_eq!(doc.full_text(), "ab", "one undo must precisely revert the augmentation");
        doc.undo();
        assert_eq!(doc.full_text(), "", "typing before augmentation was a single entry");
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
