use std::fs;
use std::time::Duration;

use appkit_core::workspace::types::TabIdAllocator;
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use notora_app::document_registry::DocumentRegistry;
use notora_core::{
    Catalog, DocumentIdentity, NavigationScope, NoteId, WORKSPACE_METADATA_DIRECTORY_NAME,
    Workspace, scan_workspace,
};
use rusqlite::Connection;
use ui::core::Widget;
use ui::virtual_card_list::{
    CardInput, CardKey, CardSelection, VirtualCardListInput, VirtualCardListWidget,
};
use ui::{LayoutCtx, NoopMeasure, Rect};

const BENCH_NOTE_COUNT: usize = 10_000;
const FIRST_PAGE_SIZE: usize = 50;
const BENCHMARK_CATALOG_FILE_NAME: &str = "catalog.sqlite3";

fn benchmark_catalog() -> (tempfile::TempDir, Catalog) {
    let directory = tempfile::tempdir().expect("benchmark directory should be created");
    let workspace = Workspace::open_or_initialize(directory.path())
        .expect("benchmark workspace should initialize");
    let notes_directory = workspace.root().join("notes");
    fs::create_dir_all(&notes_directory).expect("benchmark notes directory should exist");
    for index in 0..BENCH_NOTE_COUNT {
        let title = if index % 2 == 0 {
            format!("中文计划 {index}")
        } else {
            format!("English roadmap {index}")
        };
        fs::write(
            notes_directory.join(format!("{index:05}.md")),
            format!("# {title}\n\n摘要 {index}，正文内容 {index}"),
        )
        .expect("benchmark note should write");
    }
    let catalog = Catalog::open(&workspace.metadata_directory().join(BENCHMARK_CATALOG_FILE_NAME))
        .expect("benchmark catalog should initialize");
    scan_workspace(&workspace, &catalog).expect("benchmark catalog should populate");
    (directory, catalog)
}

fn unindexed_workspace_fixture() -> (tempfile::TempDir, Workspace, Catalog) {
    let directory = tempfile::tempdir().expect("benchmark workspace directory should be created");
    let workspace = Workspace::open_or_initialize(directory.path())
        .expect("benchmark workspace should initialize");
    let notes_directory = workspace.root().join("notes");
    fs::create_dir_all(&notes_directory).expect("benchmark notes directory should exist");
    for index in 0..BENCH_NOTE_COUNT {
        fs::write(
            notes_directory.join(format!("{index:05}.md")),
            format!("# Benchmark {index}\n\nSynthetic workspace scan content."),
        )
        .expect("benchmark note should write");
    }
    let catalog = Catalog::open(&workspace.metadata_directory().join(BENCHMARK_CATALOG_FILE_NAME))
        .expect("benchmark catalog should initialize");
    (directory, workspace, catalog)
}

fn benchmark_library_queries(criterion: &mut Criterion) {
    let (_directory, catalog) = benchmark_catalog();
    criterion.bench_function("library/search_first_page_10k", |bencher| {
        bencher.iter(|| {
            let page = catalog
                .query_catalog_cards(
                    &NavigationScope::Search { query: "中文".to_owned() },
                    None,
                    FIRST_PAGE_SIZE,
                )
                .expect("benchmark search should succeed");
            std::hint::black_box(page);
        });
    });
    criterion.bench_function("library/workspace_page_10k", |bencher| {
        bencher.iter(|| {
            let page = catalog
                .query_catalog_cards(&NavigationScope::WorkspaceRoot, None, FIRST_PAGE_SIZE)
                .expect("benchmark page query should succeed");
            std::hint::black_box(page);
        });
    });
}

fn benchmark_catalog_open(criterion: &mut Criterion) {
    let (directory, catalog) = benchmark_catalog();
    let catalog_path =
        directory.path().join(WORKSPACE_METADATA_DIRECTORY_NAME).join(BENCHMARK_CATALOG_FILE_NAME);
    criterion.bench_function("library/catalog_open_10k", |bencher| {
        bencher.iter(|| {
            let reopened_catalog =
                Catalog::open(&catalog_path).expect("benchmark catalog should reopen");
            std::hint::black_box(reopened_catalog);
        });
    });
    std::hint::black_box(catalog);
}

fn benchmark_catalog_integrity_checks(criterion: &mut Criterion) {
    let (directory, catalog) = benchmark_catalog();
    let catalog_path =
        directory.path().join(WORKSPACE_METADATA_DIRECTORY_NAME).join(BENCHMARK_CATALOG_FILE_NAME);
    let connection = Connection::open(catalog_path).expect("benchmark catalog should reopen");
    for (benchmark_name, pragma) in [
        ("library/catalog_integrity_check_10k", "PRAGMA integrity_check"),
        ("library/catalog_quick_check_10k", "PRAGMA quick_check"),
    ] {
        criterion.bench_function(benchmark_name, |bencher| {
            bencher.iter(|| {
                let diagnostic: String = connection
                    .query_row(pragma, [], |row| row.get(0))
                    .expect("benchmark integrity diagnostic should load");
                std::hint::black_box(diagnostic);
            });
        });
    }
    std::hint::black_box(catalog);
}

fn benchmark_workspace_scan(criterion: &mut Criterion) {
    let (_directory, workspace, catalog) = unindexed_workspace_fixture();
    let _ = scan_workspace(&workspace, &catalog).expect("initial benchmark scan should succeed");

    criterion.bench_function("library/workspace_scan_10k", |bencher| {
        bencher.iter(|| {
            let completion = scan_workspace(&workspace, &catalog)
                .expect("repeated benchmark scan should succeed");
            std::hint::black_box(completion);
        });
    });
}

fn benchmark_workspace_initial_scan(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("library");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(100));
    group.measurement_time(Duration::from_secs(1));
    group.bench_function("workspace_initial_scan_10k", |bencher| {
        bencher.iter_batched(
            unindexed_workspace_fixture,
            |(_directory, workspace, catalog)| {
                let completion = scan_workspace(&workspace, &catalog)
                    .expect("initial benchmark scan should succeed");
                std::hint::black_box(completion);
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

fn benchmark_tab_registry_switch(criterion: &mut Criterion) {
    const BENCH_RUNTIME_TAB_COUNT: usize = 12;
    let mut tabs = TabIdAllocator::new();
    let mut registry = DocumentRegistry::default();
    let tab_ids = (0..BENCH_RUNTIME_TAB_COUNT)
        .map(|_| {
            let tab_id = tabs.allocate();
            let _ = registry.register(DocumentIdentity::Note(NoteId::generate()), tab_id);
            tab_id
        })
        .collect::<Vec<_>>();
    let mut next_tab_index = 0usize;

    criterion.bench_function("library/tab_registry_switch_12", |bencher| {
        bencher.iter(|| {
            let tab_id = tab_ids[next_tab_index];
            next_tab_index = (next_tab_index + 1) % tab_ids.len();
            std::hint::black_box(registry.touch_tab(tab_id));
        });
    });
}

fn benchmark_virtual_card_layout(criterion: &mut Criterion) {
    let cards = (0..BENCH_NOTE_COUNT)
        .map(|index| CardInput {
            key: CardKey(index as u64),
            title: format!("Card {index}"),
            excerpt: "Precomputed excerpt".to_owned(),
            timestamp: "Modified".to_owned(),
            icon: Some("file-text".to_owned()),
            tag_summary: "#plan".to_owned(),
            selection: CardSelection::Unselected,
            closable: false,
        })
        .collect::<Vec<_>>();
    let theme = ui::theme::test_theme();
    criterion.bench_function("library/visible_card_layout_10k", |bencher| {
        bencher.iter(|| {
            let mut widget = VirtualCardListWidget::new();
            widget.set_input(VirtualCardListInput {
                cards: cards.clone(),
                scroll_offset_px: 50_000.0,
            });
            let mut measure = NoopMeasure;
            let mut layout_context =
                LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
            widget.set_rect(Rect::new(0.0, 0.0, 360.0, 640.0), &mut layout_context);
            std::hint::black_box(widget.layout());
        });
    });
}

criterion_group!(
    name = library_benches;
    config = Criterion::default().measurement_time(Duration::from_secs(3));
    targets = benchmark_library_queries, benchmark_catalog_open, benchmark_catalog_integrity_checks, benchmark_workspace_scan, benchmark_workspace_initial_scan, benchmark_tab_registry_switch, benchmark_virtual_card_layout
);
criterion_main!(library_benches);
