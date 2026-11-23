use super::{MarkdownEditorView, MarkdownView, PluginQuery, PluginResponse, ViewPlugin};
use crate::parser::MARKDOWN_PARSE_COUNT;
use textora_core::document::StringDocView;
use ui::plugin::{EditIntent, EditPlan, EditRequest, EditSelection, EditTransaction};

const EDIT_SOURCE: &str = "普通段落\n\n| first | second |";
const SOURCE_GENERATION: u32 = 1;

fn render(view: &mut MarkdownEditorView) {
    let mut shaper = shaping::Shaper::new().expect("test font shaper must initialize");
    let source = view.source.clone();
    view.render(
        &StringDocView::new(&source),
        ui::Rect::new(0.0, 0.0, 800.0, 600.0),
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
) {
    let mut edited = EDIT_SOURCE.to_owned();
    for replacement in transaction.replacements.into_iter().rev() {
        edited.replace_range(replacement.range, &replacement.text);
    }
    assert_eq!(edited.as_bytes(), expected_source.as_bytes());
    assert_eq!(transaction.selection_after, EditSelection::Caret(expected_caret));

    let (_, sync_parses) = count_parses(|| {
        view.set_source(edited, transaction.source_generation + 1);
    });
    let (_, frame_parses) = count_parses(|| {
        view.engine.handle_set_cursor_byte(expected_caret);
        render(view);
    });
    assert_eq!(sync_parses, 1, "source synchronization");
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
    let (plan, parses) = count_parses(|| view.edit_policy().plan_edit(&request));
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
    assert_committed_edit(&mut view, transaction, "普通新段落\n\n| first | second |", caret_after);
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
    assert_committed_edit(&mut view, transaction, "\u{a0}新\n\n\n| first | second |", caret_after);
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
    assert_committed_edit(&mut view, transaction, expected_source, caret_after);
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
fn host_queries_use_the_current_snapshot_parse() {
    let mut view = MarkdownEditorView::new();
    let source = "| first | second |\n| --- | --- |\n| left | right |";
    let doc = StringDocView::new(source);
    view.set_source(source.into(), 1);
    render(&mut view);

    MARKDOWN_PARSE_COUNT.with(|count| count.set(0));
    assert!(matches!(
        view.query(
            PluginQuery::TableStructureCapabilities { source_generation: 1, cursor_byte: 2 },
            &doc
        ),
        PluginResponse::TableStructureCapabilities(Some(ui::plugin::TableStructureCapabilities {
            can_delete_row: false,
            can_delete_column: true,
        }))
    ));
    assert!(matches!(
        view.query(PluginQuery::SourceSpacingProtectedRanges { source_generation: 1 }, &doc),
        PluginResponse::SourceSpacingProtectedRanges(ranges) if !ranges.is_empty()
    ));
    assert!(matches!(
        view.query(PluginQuery::SourceSpacingProtectedRanges { source_generation: 2 }, &doc),
        PluginResponse::None
    ));
    assert_eq!(MARKDOWN_PARSE_COUNT.with(std::cell::Cell::get), 0);
}

#[test]
fn editor_snapshot_requires_initial_generation_zero_sync() {
    let mut view = MarkdownEditorView::new();
    let source = "| first | second |\n| --- | --- |\n| left | right |";
    let doc = StringDocView::new(source);
    let table_query =
        || PluginQuery::TableStructureCapabilities { source_generation: 0, cursor_byte: 2 };

    assert!(matches!(view.query(table_query(), &doc), PluginResponse::None));
    assert!(matches!(
        view.query(PluginQuery::SourceSpacingProtectedRanges { source_generation: 0 }, &doc),
        PluginResponse::None
    ));
    assert!(matches!(
        view.query(PluginQuery::NeedsSourceUpdate(0), &doc),
        PluginResponse::Bool(true)
    ));

    view.set_source(source.into(), 0);

    assert!(matches!(
        view.query(table_query(), &doc),
        PluginResponse::TableStructureCapabilities(Some(_))
    ));
    assert!(matches!(
        view.query(PluginQuery::NeedsSourceUpdate(0), &doc),
        PluginResponse::Bool(false)
    ));
}

#[test]
fn preview_snapshot_requires_initial_generation_zero_sync() {
    let mut view = MarkdownView::new();
    let source = "`inline code`";
    let doc = StringDocView::new(source);
    let spacing_query = || PluginQuery::SourceSpacingProtectedRanges { source_generation: 0 };

    assert!(matches!(view.query(spacing_query(), &doc), PluginResponse::None));
    assert!(matches!(
        view.query(PluginQuery::NeedsSourceUpdate(0), &doc),
        PluginResponse::Bool(true)
    ));

    view.set_source(source.into(), 0);

    assert!(matches!(
        view.query(spacing_query(), &doc),
        PluginResponse::SourceSpacingProtectedRanges(ranges) if !ranges.is_empty()
    ));
    assert!(matches!(
        view.query(PluginQuery::NeedsSourceUpdate(0), &doc),
        PluginResponse::Bool(false)
    ));
}

#[test]
fn semantic_edit_query_rejects_stale_snapshot() {
    use ui::plugin::{SemanticEditCommand, SemanticEditPlan};

    let mut view = MarkdownEditorView::new();
    view.set_source("old".into(), 1);
    let doc = StringDocView::new("new");
    let query = || PluginQuery::PlanSemanticEdit {
        command: SemanticEditCommand::SetHeadingLevel(2),
        source_generation: 2,
        cursor_byte: 0,
        selection: None,
    };

    assert!(matches!(view.query(query(), &doc), PluginResponse::None));

    view.set_source("new".into(), 2);

    assert!(
        matches!(view.query(query(), &doc), PluginResponse::SemanticEdit(SemanticEditPlan::Apply(transaction))
        if transaction.replacements[0].text == "## new")
    );
}

#[test]
fn layout_reflow_reuses_the_current_source_parse() {
    let mut view = MarkdownEditorView::new();
    view.set_source(EDIT_SOURCE.into(), SOURCE_GENERATION);
    render(&mut view);

    MARKDOWN_PARSE_COUNT.with(|count| count.set(0));
    view.engine.base_font_size *= 2.0;
    render(&mut view);
    assert_eq!(MARKDOWN_PARSE_COUNT.with(std::cell::Cell::get), 0);
}
