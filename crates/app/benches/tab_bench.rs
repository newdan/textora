use criterion::{Criterion, criterion_group, criterion_main};
use std::io::Write;
use std::time::{Duration, Instant};
use textora_app::dev_support::DocumentView;
use ui::tab_bar::{TabBarWidget, TabBarWidgetInput, TabInfo, tab_bar_height};

/// Benchmark: layout 100 tabs.
fn bench_tab_layout_100(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let mut paths = Vec::new();
    for i in 0..100 {
        let path = dir.path().join(format!("tab_{i}.txt"));
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, "File {i}: quick brown fox").unwrap();
        file.flush().unwrap();
        paths.push(path);
    }

    let tab_infos: Vec<TabInfo> = paths
        .iter()
        .map(|p| TabInfo {
            title: p.file_name().unwrap().to_str().unwrap_or("?").to_string(),
            file_path: Some(p.clone()),
            is_dirty: false,
            language: "text".to_string(),
            pinned: false,
        })
        .collect();

    let ctx = ui::tab_bar::TabBarCtx { screen_w: 1920.0, screen_h: 1080.0, dpi: 1.0 };
    let _tab_height = tab_bar_height(1.0);

    let metrics = ui::settings::UiMetrics {
        dpi: 1.0,
        font_size: 15.0,
        line_height: 20.0,
        status_bar_height: 0.0,
        gutter_padding: 0.0,
        toc_width: 200.0,
        content_left_margin: 32.0,
        scrollbar_reserve: ui::scrollbar::SCROLLBAR_RESERVE_PX,
        show_line_numbers: false,
        show_status_bar: false,
    };
    c.bench_function("tab_layout_100", |b| {
        b.iter(|| {
            let mut widget = TabBarWidget::new();
            widget.set_input(
                TabBarWidgetInput {
                    tabs: tab_infos.clone(),
                    active_index: Some(0),
                    back_enabled: false,
                    forward_enabled: false,
                    screen_size_px: (ctx.screen_w, ctx.screen_h),
                    hovered_index: None,
                    scroll_offset_px: 0.0,
                    metrics,
                },
                None,
            );
            std::hint::black_box(&widget);
        });
    });

    let start = Instant::now();
    for _ in 0..1000 {
        let mut widget = TabBarWidget::new();
        widget.set_input(
            TabBarWidgetInput {
                tabs: tab_infos.clone(),
                active_index: Some(0),
                back_enabled: false,
                forward_enabled: false,
                screen_size_px: (ctx.screen_w, ctx.screen_h),
                hovered_index: None,
                scroll_offset_px: 0.0,
                metrics,
            },
            None,
        );
        std::hint::black_box(&widget);
    }
    let avg = start.elapsed() / 1000;
    eprintln!("[tab_layout_100] avg={avg:?} (target < 4ms)");
    assert!(avg < Duration::from_millis(4), "tab layout too slow: {avg:?}");
}

fn bench_tab_switch(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let mut paths = Vec::new();
    for i in 0..10 {
        let path = dir.path().join(format!("switch_{i}.txt"));
        let mut file = std::fs::File::create(&path).unwrap();
        for j in 0..100 {
            writeln!(file, "Line {j}: The quick brown fox jumps over the lazy dog. Extra padding.")
                .unwrap();
        }
        file.flush().unwrap();
        paths.push(path);
    }

    let doc_views: Vec<_> =
        paths.iter().map(|p| DocumentView::from_file(p, 40, 10.0).expect("load")).collect();

    let mut active = 0usize;

    c.bench_function("tab_switch", |b| {
        b.iter(|| {
            active = (active + 1) % doc_views.len();
            let dv = &doc_views[active];
            std::hint::black_box(dv.line_count());
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50);
    targets = bench_tab_layout_100, bench_tab_switch
}
criterion_main!(benches);
