use std::time::{Duration, UNIX_EPOCH};

use notora_core::catalog::SearchIndexEntry;
use notora_core::{Catalog, CatalogNote, DocumentKind, NavigationScope, NoteId};
use ui::core::Widget;
use ui::virtual_card_list::{
    CardIconTone, CardInput, CardKey, CardSelection, VirtualCardListInput, VirtualCardListWidget,
};
use ui::{LayoutCtx, NoopMeasure, Rect};

const NOTE_FIXTURE_COUNT: usize = 10_000;
const CARD_VIEWPORT_HEIGHT_PX: f32 = 640.0;

#[test]
fn searches_and_virtualizes_a_ten_thousand_note_library() {
    let directory = tempfile::tempdir().expect("fixture directory should be created");
    let catalog = Catalog::open(&directory.path().join("catalog.sqlite3"))
        .expect("fixture catalog should initialize");
    let mut search_entries = Vec::with_capacity(NOTE_FIXTURE_COUNT);
    for index in 0..NOTE_FIXTURE_COUNT {
        let note_id = NoteId::generate();
        let title = if index % 2 == 0 {
            format!("中文计划 {index}")
        } else {
            format!("English roadmap {index}")
        };
        let relative_path = format!("notes/{index:05}.md");
        catalog
            .upsert_active_note(&CatalogNote {
                note_id,
                relative_path: relative_path.clone().into(),
                kind: DocumentKind::Markdown,
                title: title.clone(),
                excerpt: format!("摘要 {index}"),
                modified_at: UNIX_EPOCH + Duration::from_secs(index as u64),
                file_size: 1,
                content_hash: (index as u64).to_le_bytes().to_vec(),
                starred: false,
            })
            .expect("fixture note should persist");
        search_entries.push(SearchIndexEntry {
            note_id,
            title,
            relative_path: relative_path.into(),
            body: format!("正文内容 {index}"),
            tags: vec!["计划".to_owned()],
        });
    }
    catalog.index_note_batch(&search_entries).expect("fixture search index should persist");

    let search_page = catalog
        .query_catalog_cards(&NavigationScope::Search { query: "中文".to_owned() }, None, 50)
        .expect("Chinese search should return a page");
    assert_eq!(search_page.cards.len(), 50);
    assert!(search_page.next_cursor.is_some());
    assert!(search_page.cards.iter().all(|card| card.title.starts_with("中文计划")));

    let cards = (0..NOTE_FIXTURE_COUNT)
        .map(|index| CardInput {
            key: CardKey(index as u64),
            title: format!("Card {index}"),
            excerpt: "预计算摘要".to_owned(),
            timestamp: "Modified".to_owned(),
            icon: Some("file-text".to_owned()),
            icon_tone: CardIconTone::Neutral,
            tag_summary: "#计划".to_owned(),
            selection: CardSelection::Unselected,
            closable: false,
        })
        .collect();
    let mut widget = VirtualCardListWidget::new();
    widget.set_input(VirtualCardListInput { cards, scroll_offset_px: 50_000.0 });
    let theme = ui::theme::test_theme();
    let mut measure = NoopMeasure;
    let mut layout_context =
        LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    widget.set_rect(Rect::new(0.0, 0.0, 360.0, CARD_VIEWPORT_HEIGHT_PX), &mut layout_context);

    let layout = widget.layout();
    assert_eq!(layout.card_count, NOTE_FIXTURE_COUNT);
    assert!(layout.visible_range.len() < 20, "only viewport cards plus overscan may be laid out");
}
