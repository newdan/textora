use super::{MarkdownEditorView, MarkdownView, PluginQuery, PluginResponse, ViewPlugin};
use crate::parser::MARKDOWN_PARSE_COUNT;
use textora_core::document::StringDocView;

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
    let mut view = MarkdownEditorView::new();
    let source = "| first | second |";
    view.set_source(source.into(), 1);
    view.engine.handle_set_cursor_byte(source.len());
    render(&mut view);

    MARKDOWN_PARSE_COUNT.with(|count| count.set(0));
    let plan = ui::plugin::EditPolicy::plan_edit(
        &view,
        &ui::plugin::EditRequest {
            source_generation: 1,
            cursor_byte: source.len(),
            selection: None,
            intent: ui::plugin::EditIntent::InsertParagraphBreak,
        },
    );
    assert!(matches!(plan, ui::plugin::EditPlan::Apply(_)));
    assert_eq!(MARKDOWN_PARSE_COUNT.with(std::cell::Cell::get), 1);
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
    view.set_source("First paragraph\n\nSecond paragraph".into(), 1);
    render(&mut view);

    MARKDOWN_PARSE_COUNT.with(|count| count.set(0));
    view.engine.base_font_size *= 2.0;
    render(&mut view);
    assert_eq!(MARKDOWN_PARSE_COUNT.with(std::cell::Cell::get), 0);
}
