use super::{MarkdownEditorView, MarkdownView, ViewPlugin, ViewPluginCapabilities};
use crate::parser::MARKDOWN_PARSE_COUNT;
use textora_core::document::StringDocView;
use ui::plugin::{
    EditIntent, EditPlan, EditPolicy, EditRequest, EditSelection, EditTransaction, EditableView,
    SyncOutcome, SyncReason,
};

const EDIT_SOURCE: &str = "普通段落\n\n| first | second |";
const SOURCE_GENERATION: u32 = 1;

fn render(view: &mut MarkdownEditorView) {
    render_at_width(view, 800.0);
}

fn render_at_width(view: &mut MarkdownEditorView, width: f32) {
    let mut shaper = shaping::Shaper::new().expect("test font shaper must initialize");
    let source = view.source.clone();
    view.render(
        &StringDocView::new(&source),
        ui::Rect::new(0.0, 0.0, width, 600.0),
        &ui::theme::test_theme(),
        &mut shaper,
        1.0,
    );
}

fn count_parses<T>(operation: impl FnOnce() -> T) -> (T, usize) {
    MARKDOWN_PARSE_COUNT.with(|count| count.set(0));
    let output = operation();
    (output, MARKDOWN_PARSE_COUNT.with(std::cell::Cell::get))
}

fn assert_committed_edit(
    view: &mut MarkdownEditorView,
    transaction: EditTransaction,
    expected_source: &str,
    expected_caret: usize,
    expected_sync_parses: usize,
) {
    let mut edited = EDIT_SOURCE.to_owned();
    for replacement in transaction.replacements.into_iter().rev() {
        edited.replace_range(replacement.range, &replacement.text);
    }
    assert_eq!(edited.as_bytes(), expected_source.as_bytes());
    assert_eq!(transaction.selection_after, EditSelection::Caret(expected_caret));

    let (_, sync_parses) = count_parses(|| {
        assert_eq!(
            view.sync_document(
                transaction.source_generation.wrapping_add(1),
                SyncReason::Normal,
                &mut || std::mem::take(&mut edited),
            ),
            SyncOutcome::Updated
        );
    });
    let (_, frame_parses) = count_parses(|| {
        view.engine.handle_set_cursor_byte(expected_caret);
        render(view);
    });
    assert_eq!(sync_parses, expected_sync_parses, "source synchronization");
    assert_eq!(frame_parses, 0, "first frame after synchronization");
    assert_eq!(view.source.as_bytes(), expected_source.as_bytes());
    assert_eq!(
        view.engine.edit_ctx.as_ref().map(|context| context.cursor_byte),
        Some(expected_caret)
    );
}

fn plan_rendered_edit(request: EditRequest) -> (MarkdownEditorView, EditPlan, usize) {
    let mut view = MarkdownEditorView::new();
    view.set_source(EDIT_SOURCE.into(), SOURCE_GENERATION);
    view.engine.handle_set_cursor_byte(request.cursor_byte);
    render(&mut view);
    let (plan, parses) = count_parses(|| view.plan_edit(&request));
    (view, plan, parses)
}

#[test]
fn ordinary_insert_uses_default_without_parsing_after_layout() {
    let caret = "普通".len();
    let (mut view, plan, parses) = plan_rendered_edit(EditRequest {
        source_generation: SOURCE_GENERATION,
        cursor_byte: caret,
        selection: None,
        intent: EditIntent::InsertText("新".into()),
    });
    assert_eq!(parses, 0);
    assert_eq!(plan, EditPlan::UseDefault);
    let caret_after = "普通新".len();
    let transaction =
        EditTransaction::replace(SOURCE_GENERATION, caret..caret, "新".into(), caret_after);
    assert_committed_edit(
        &mut view,
        transaction,
        "普通新段落\n\n| first | second |",
        caret_after,
        1,
    );
}

#[test]
fn selection_replacement_plans_and_commits_against_the_same_source() {
    let selection = 0.."普通段落".len();
    let (mut view, plan, parses) = plan_rendered_edit(EditRequest {
        source_generation: SOURCE_GENERATION,
        cursor_byte: selection.end,
        selection: Some(selection.clone()),
        intent: EditIntent::InsertText(" 新".into()),
    });
    assert_eq!(parses, 2);

    let replacement = "\u{a0}新\n";
    let caret_after = "\u{a0}新".len();
    let EditPlan::Apply(transaction) = plan else {
        panic!("leading space must be protected when replacing a paragraph selection: {plan:?}");
    };
    assert_eq!(
        transaction,
        EditTransaction::replace(SOURCE_GENERATION, selection, replacement.into(), caret_after)
    );
    assert_committed_edit(
        &mut view,
        transaction,
        "\u{a0}新\n\n\n| first | second |",
        caret_after,
        1,
    );
}

#[test]
fn source_snapshot_shares_text_between_editor_and_engine() {
    let mut view = MarkdownEditorView::new();
    let text = String::from("shared text");
    let text_ptr = text.as_ptr();
    view.set_source(text, 1);

    let engine_source = view.engine.edit_source.as_deref().expect("editor owns a source");
    assert_eq!(view.source.as_ptr(), text_ptr);
    assert_eq!(engine_source.as_ptr(), text_ptr);
    assert_eq!(view.source.parsed().source.as_ptr(), text_ptr);
}

#[test]
fn table_enter_parses_only_the_candidate_after_layout() {
    let caret = EDIT_SOURCE.len();
    let (mut view, plan, parses) = plan_rendered_edit(EditRequest {
        source_generation: SOURCE_GENERATION,
        cursor_byte: caret,
        selection: None,
        intent: EditIntent::InsertParagraphBreak,
    });
    assert_eq!(parses, 1);

    let header_start = "普通段落\n\n".len();
    let expected_source = "普通段落\n\n| first | second |\n| --- | --- |\n|  |  |";
    let caret_after = "普通段落\n\n| first | second |\n| --- | --- |\n| ".len();
    let EditPlan::Apply(transaction) = plan else {
        panic!("table Enter must create a table transaction");
    };
    assert_eq!(
        transaction,
        EditTransaction::replace(
            SOURCE_GENERATION,
            header_start..caret,
            expected_source[header_start..].into(),
            caret_after,
        )
    );
    assert_committed_edit(&mut view, transaction, expected_source, caret_after, 0);
}

const TABLE_RESULT: &str = "普通段落\n\n| first | second |\n| --- | --- |\n|  |  |";

fn planned_table_enter(generation: u32) -> MarkdownEditorView {
    let mut view = MarkdownEditorView::new();
    view.set_source(EDIT_SOURCE.into(), generation);
    view.set_cursor_byte(EDIT_SOURCE.len());
    render(&mut view);
    let (plan, parses) = count_parses(|| {
        view.plan_edit(&EditRequest {
            source_generation: generation,
            cursor_byte: EDIT_SOURCE.len(),
            selection: None,
            intent: EditIntent::InsertParagraphBreak,
        })
    });
    assert!(matches!(plan, EditPlan::Apply(_)));
    assert_eq!(parses, 1);
    assert_eq!(view.source.as_str(), EDIT_SOURCE);
    view
}

fn assert_snapshot_sync_parses(
    view: &mut MarkdownEditorView,
    text: &str,
    generation: u32,
    reason: SyncReason,
    expected_parses: usize,
) {
    let (outcome, parses) =
        count_parses(|| view.sync_document(generation, reason, &mut || text.into()));
    assert_eq!(outcome, SyncOutcome::Updated);
    assert_eq!(parses, expected_parses);
    assert_eq!(view.source.as_bytes(), text.as_bytes());
}

#[test]
fn table_candidate_reuse_requires_exact_text_and_next_generation() {
    let mut view = planned_table_enter(SOURCE_GENERATION);
    let same_length_edit = TABLE_RESULT.replace("first", "other");
    assert_eq!(same_length_edit.len(), TABLE_RESULT.len());
    assert_snapshot_sync_parses(
        &mut view,
        &same_length_edit,
        SOURCE_GENERATION + 1,
        SyncReason::Normal,
        1,
    );

    let mut view = planned_table_enter(SOURCE_GENERATION);
    assert_snapshot_sync_parses(
        &mut view,
        TABLE_RESULT,
        SOURCE_GENERATION + 2,
        SyncReason::Normal,
        1,
    );
}

#[test]
fn table_candidate_can_be_consumed_after_an_unchanged_sync_and_generation_wrap() {
    let mut view = planned_table_enter(u32::MAX);
    assert_eq!(
        view.sync_document(u32::MAX, SyncReason::Normal, &mut || panic!("unchanged source")),
        SyncOutcome::Unchanged
    );
    assert_snapshot_sync_parses(&mut view, TABLE_RESULT, 0, SyncReason::Normal, 0);
}

#[test]
fn forced_snapshot_sync_drops_unconfirmed_table_candidate() {
    let mut view = planned_table_enter(SOURCE_GENERATION);
    assert_snapshot_sync_parses(&mut view, EDIT_SOURCE, SOURCE_GENERATION, SyncReason::Force, 0);
    assert_snapshot_sync_parses(
        &mut view,
        TABLE_RESULT,
        SOURCE_GENERATION + 1,
        SyncReason::Normal,
        1,
    );

    let mut view = planned_table_enter(SOURCE_GENERATION);
    assert_snapshot_sync_parses(
        &mut view,
        TABLE_RESULT,
        SOURCE_GENERATION + 1,
        SyncReason::Force,
        1,
    );
}

#[test]
fn forced_history_sync_does_not_bring_back_consumed_table_candidate() {
    let mut view = planned_table_enter(SOURCE_GENERATION);
    assert_snapshot_sync_parses(
        &mut view,
        TABLE_RESULT,
        SOURCE_GENERATION + 1,
        SyncReason::Normal,
        0,
    );
    assert_snapshot_sync_parses(
        &mut view,
        EDIT_SOURCE,
        SOURCE_GENERATION + 2,
        SyncReason::Force,
        1,
    );
    assert_snapshot_sync_parses(
        &mut view,
        TABLE_RESULT,
        SOURCE_GENERATION + 3,
        SyncReason::Force,
        1,
    );
}

#[test]
fn another_edit_request_drops_unconfirmed_table_candidate() {
    let mut view = planned_table_enter(SOURCE_GENERATION);
    assert_eq!(
        view.plan_edit(&EditRequest {
            source_generation: SOURCE_GENERATION,
            cursor_byte: "普通".len(),
            selection: None,
            intent: EditIntent::InsertText("新".into()),
        }),
        EditPlan::UseDefault,
    );
    assert_snapshot_sync_parses(
        &mut view,
        TABLE_RESULT,
        SOURCE_GENERATION + 1,
        SyncReason::Normal,
        1,
    );
}

#[test]
fn replacement_preedit_does_not_consume_table_candidate_even_when_text_matches() {
    let mut view = planned_table_enter(SOURCE_GENERATION);
    view.engine.set_sel_anchor_byte(Some(0));
    view.engine.set_sel_cursor_byte(Some(EDIT_SOURCE.len()));
    view.set_preedit(TABLE_RESULT.into(), Some((TABLE_RESULT.len(), TABLE_RESULT.len())));
    let (_, preedit_parses) = count_parses(|| render(&mut view));
    assert_eq!(preedit_parses, 1);
    assert!(view.engine.replacement_preedit.is_some());
    assert_eq!(view.source.as_str(), EDIT_SOURCE);

    view.set_preedit(String::new(), None);
    assert!(view.engine.replacement_preedit.is_none());
    assert_snapshot_sync_parses(
        &mut view,
        TABLE_RESULT,
        SOURCE_GENERATION + 1,
        SyncReason::Normal,
        1,
    );
}

#[test]
fn enter_with_whole_paragraph_selected_parses_erased_candidate_once() {
    let mut view = MarkdownEditorView::new();
    let source = "text";
    view.set_source(source.into(), 1);
    render(&mut view);

    MARKDOWN_PARSE_COUNT.with(|count| count.set(0));
    let plan = ui::plugin::EditPolicy::plan_edit(
        &view,
        &ui::plugin::EditRequest {
            source_generation: 1,
            cursor_byte: source.len(),
            selection: Some(0..source.len()),
            intent: ui::plugin::EditIntent::InsertParagraphBreak,
        },
    );
    assert!(matches!(plan, ui::plugin::EditPlan::Apply(_)));
    assert_eq!(MARKDOWN_PARSE_COUNT.with(std::cell::Cell::get), 1);
}

#[test]
fn selection_replacement_reuses_the_erased_candidate_in_fallback_rules() {
    let mut view = MarkdownEditorView::new();
    let source = "text";
    view.set_source(source.into(), 1);
    render(&mut view);

    for replacement in ["a\nb", " x"] {
        MARKDOWN_PARSE_COUNT.with(|count| count.set(0));
        ui::plugin::EditPolicy::plan_edit(
            &view,
            &ui::plugin::EditRequest {
                source_generation: 1,
                cursor_byte: source.len(),
                selection: Some(0..source.len()),
                intent: ui::plugin::EditIntent::InsertText(replacement.to_owned()),
            },
        );
        assert_eq!(MARKDOWN_PARSE_COUNT.with(std::cell::Cell::get), 1, "{replacement:?}");
    }
}

#[test]
fn typed_edit_queries_use_the_current_snapshot_parse() {
    let mut view = MarkdownEditorView::new();
    let source = "| first | second |\n| --- | --- |\n| left | right |";
    view.set_source(source.into(), 1);
    render(&mut view);

    MARKDOWN_PARSE_COUNT.with(|count| count.set(0));
    assert_eq!(
        view.table_structure_capabilities(1, 2),
        Some(ui::plugin::TableStructureCapabilities {
            can_delete_row: false,
            can_delete_column: true,
        })
    );
    assert!(view.source_spacing_protected_ranges(1).is_some_and(|ranges| !ranges.is_empty()));
    assert!(view.source_spacing_protected_ranges(2).is_none());
    assert_eq!(MARKDOWN_PARSE_COUNT.with(std::cell::Cell::get), 0);
}

#[test]
fn editor_snapshot_requires_initial_generation_zero_sync() {
    let mut view = MarkdownEditorView::new();
    let source = "| first | second |\n| --- | --- |\n| left | right |";

    assert!(view.table_structure_capabilities(0, 2).is_none());
    assert!(view.source_spacing_protected_ranges(0).is_none());
    assert!(view.needs_source_update(0));

    view.set_source(source.into(), 0);

    assert!(view.table_structure_capabilities(0, 2).is_some());
    assert!(!view.needs_source_update(0));
}

#[test]
fn preview_snapshot_requires_initial_generation_zero_sync() {
    let mut view = MarkdownView::new();
    let source = "`inline code`";

    assert!(view.source_spacing_protected_ranges(0).is_none());
    assert!(view.needs_source_update(0));

    view.set_source(source.into(), 0);

    assert!(view.source_spacing_protected_ranges(0).is_some_and(|ranges| !ranges.is_empty()));
    assert!(!view.needs_source_update(0));
}

#[test]
fn semantic_edit_rejects_stale_snapshot() {
    use ui::plugin::{SemanticEditCommand, SemanticEditPlan};

    let mut view = MarkdownEditorView::new();
    view.set_source("old".into(), 1);
    let plan = |view: &MarkdownEditorView| {
        view.plan_semantic_edit(SemanticEditCommand::SetHeadingLevel(2), 2, 0, None)
    };

    assert_eq!(plan(&view), SemanticEditPlan::Unsupported);

    view.set_source("new".into(), 2);

    assert!(matches!(
        plan(&view), SemanticEditPlan::Apply(transaction)
        if transaction.replacements[0].text == "## new"
    ));
}

#[test]
fn layout_reflow_reuses_the_current_source_parse() {
    use crate::source_snapshot::SOURCE_LINE_MAP_BUILD_COUNT;

    SOURCE_LINE_MAP_BUILD_COUNT.with(|count| count.set(0));
    let mut view = MarkdownEditorView::new();
    view.set_source(EDIT_SOURCE.into(), SOURCE_GENERATION);
    render(&mut view);
    assert_eq!(SOURCE_LINE_MAP_BUILD_COUNT.with(std::cell::Cell::get), 1);

    MARKDOWN_PARSE_COUNT.with(|count| count.set(0));
    view.engine.base_font_size *= 2.0;
    render(&mut view);
    assert_eq!(MARKDOWN_PARSE_COUNT.with(std::cell::Cell::get), 0);
    assert_eq!(SOURCE_LINE_MAP_BUILD_COUNT.with(std::cell::Cell::get), 1);
}

#[test]
fn width_reflow_reuses_editor_structure_and_matches_cold_projection() {
    use crate::builder::MARKDOWN_STRUCTURE_BUILD_COUNT;

    const NARROW_WIDTH: f32 = 220.0;
    const SOURCE: &str = "# 标题\n\n普通段落和 **hello world** 应该正常重排。\n\n| a | b |\n|---|---|\n| c | d |\n\n~~~rust\nfn main() {}\n~~~";
    let make_editor = || {
        let mut view = MarkdownEditorView::new();
        view.set_source(SOURCE.into(), SOURCE_GENERATION);
        view.set_cursor_byte(SOURCE.find("hello").expect("fixture has emphasis"));
        view
    };

    let mut reflowed = make_editor();
    MARKDOWN_STRUCTURE_BUILD_COUNT.with(|count| count.set(0));
    render(&mut reflowed);
    assert_eq!(MARKDOWN_STRUCTURE_BUILD_COUNT.with(std::cell::Cell::get), 1);

    let (_, reflow_parses) = count_parses(|| render_at_width(&mut reflowed, NARROW_WIDTH));
    assert_eq!(reflow_parses, 0);
    assert_eq!(MARKDOWN_STRUCTURE_BUILD_COUNT.with(std::cell::Cell::get), 1);

    let mut cold = make_editor();
    render_at_width(&mut cold, NARROW_WIDTH);
    let reflowed_lines = reflowed.engine.flat_lines();
    let cold_lines = cold.engine.flat_lines();
    assert_eq!(reflowed_lines.len(), cold_lines.len());
    for (reflowed_line, cold_line) in reflowed_lines.iter().zip(cold_lines) {
        assert_eq!(reflowed_line.text, cold_line.text);
        assert_eq!(reflowed_line.rect, cold_line.rect);
        assert_eq!(reflowed_line.source_projection, cold_line.source_projection);
        assert_eq!(reflowed_line.atomic_source_range, cold_line.atomic_source_range);
    }
    assert_eq!(reflowed.engine.content_height, cold.engine.content_height);
    assert_eq!(reflowed.engine.cursor_screen_pos(), cold.engine.cursor_screen_pos());
}

#[test]
fn document_sync_loads_only_initial_or_forced_markdown_snapshots() {
    use ui::plugin::{SyncOutcome, SyncReason};

    let mut editor = MarkdownEditorView::new();
    let mut preview = MarkdownView::new();
    for plugin in [&mut editor as &mut dyn ViewPlugin, &mut preview] {
        assert_eq!(
            plugin.sync_document(0, SyncReason::Normal, &mut || "first".into()),
            SyncOutcome::Updated
        );
        assert_eq!(
            plugin.sync_document(0, SyncReason::Normal, &mut || panic!("current snapshot")),
            SyncOutcome::Unchanged
        );
        assert_eq!(
            plugin.sync_document(0, SyncReason::Force, &mut || "replacement".into()),
            SyncOutcome::Updated
        );
    }
    assert_eq!(editor.source.as_str(), "replacement");
}

#[test]
fn novel_document_sync_never_loads_the_document_snapshot() {
    use ui::plugin::SyncReason;

    let mut novel = super::NovelView::new();
    for reason in [SyncReason::Normal, SyncReason::Force] {
        novel.sync_document(0, reason, &mut || panic!("Novel reads through DocView"));
    }
}
