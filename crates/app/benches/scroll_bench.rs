use appkit_core::document::DocumentModel;
use appkit_shell::display_line_map::DisplayLineMap;
use appkit_shell::editor_plugin::EditorPlugin;
use appkit_shell::prepared_tab::PreparedTab;
use appkit_shell::tab_runtime::TabRuntime;
use criterion::{Criterion, criterion_group, criterion_main};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};
use ui::viewport::{ScrollAnchor, Viewport};

const BENCH_LINE_HEIGHT: f32 = 24.27;
const SCROLL_FILE_LINES: usize = 100_000;
const SCROLL_VISIBLE_ROWS: usize = 50;

fn load_scroll_fixture(path: &Path) -> (DocumentModel, DisplayLineMap, Viewport) {
    let document = DocumentModel::from_file(path).expect("load scroll benchmark fixture");
    let runtime = TabRuntime::new(Box::new(EditorPlugin::new()));
    let PreparedTab { document, runtime } = PreparedTab::new(document, runtime);
    let display_map = runtime.presentation.display.display_map;
    (document, display_map, Viewport::new(SCROLL_VISIBLE_ROWS))
}

fn scroll_one_doc_line(
    document: &DocumentModel,
    display_map: &DisplayLineMap,
    viewport: &mut Viewport,
) -> Vec<String> {
    viewport.scroll_doc_lines(1, display_map);
    viewport.derive_scroll_top(display_map, BENCH_LINE_HEIGHT);
    viewport
        .visible_doc_range_from_anchor(display_map, BENCH_LINE_HEIGHT)
        .filter_map(|line| document.document_line_bytes(line))
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .collect()
}

fn reset_scroll_at_end(viewport: &mut Viewport) {
    if viewport.scroll_anchor.doc_line > SCROLL_FILE_LINES - SCROLL_VISIBLE_ROWS {
        viewport.scroll_anchor = ScrollAnchor::top();
    }
}

fn bench_scroll_single_frame(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scroll_frame.txt");
    let mut file = std::fs::File::create(&path).unwrap();
    for i in 0..SCROLL_FILE_LINES {
        writeln!(file, "Line {i}: The quick brown fox jumps over the lazy dog.").unwrap();
    }
    file.flush().unwrap();

    let (document, display_map, mut viewport) = load_scroll_fixture(&path);

    c.bench_function("scroll_single_frame", |b| {
        b.iter(|| {
            let visible_lines = scroll_one_doc_line(&document, &display_map, &mut viewport);
            std::hint::black_box(&visible_lines);
            reset_scroll_at_end(&mut viewport);
        });
    });
}

/// Simulate 60 seconds of continuous scrolling at 60 fps.
/// Reports frame time histogram and asserts < 1% dropped frames.
fn bench_scroll_60s_60fps(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scroll_60s.txt");
    let mut file = std::fs::File::create(&path).unwrap();
    for i in 0..SCROLL_FILE_LINES {
        writeln!(file, "Line {i}: The quick brown fox jumps over the lazy dog. Extra padding to simulate realistic line length around 80 chars.").unwrap();
    }
    file.flush().unwrap();

    // Use sample_size=5 to keep total runtime reasonable (5 × ~1s each)
    let mut group = c.benchmark_group("scroll_60s");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("scroll_3600_frames", |b| {
        b.iter_custom(|_iters| {
            let (document, display_map, mut viewport) = load_scroll_fixture(&path);

            let total_frames = 3600u64; // 60s × 60fps
            let frame_budget = Duration::from_nanos(16_666_666); // ~16.67ms
            let mut dropped = 0u64;
            let mut total_time = Duration::ZERO;

            for _ in 0..total_frames {
                let frame_start = Instant::now();

                let visible_lines = scroll_one_doc_line(&document, &display_map, &mut viewport);
                std::hint::black_box(&visible_lines);

                let frame_time = frame_start.elapsed();
                total_time += frame_time;
                if frame_time > frame_budget {
                    dropped += 1;
                }

                reset_scroll_at_end(&mut viewport);
            }

            let drop_pct = dropped as f64 / total_frames as f64 * 100.0;
            eprintln!(
                "[scroll_60s] {total_frames} frames, dropped={dropped} ({drop_pct:.2}%), \
                 total={total_time:?}, avg={:?}",
                total_time / total_frames as u32
            );
            assert!(drop_pct < 1.0, "dropped frames {drop_pct:.2}% >= 1% threshold");

            total_time
        });
    });

    group.finish();
}

/// Benchmark: continuous character insertion throughput.
/// plans.md §6: 1s continuous insert ≥ 10,000 times.
fn bench_typing_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("typing");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(3));

    group.bench_function("insert_10k_chars", |b| {
        b.iter_custom(|_iters| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("typing.txt");
            std::fs::write(&path, "").unwrap();

            let mut document = DocumentModel::from_file(&path).expect("load typing fixture");

            let start = Instant::now();
            for _ in 0..10_000 {
                document.insert_at_cursor(b"a");
            }
            let elapsed = start.elapsed();

            let ops_per_sec = 10_000.0 / elapsed.as_secs_f64();
            eprintln!("[typing] 10k inserts in {elapsed:?} ({ops_per_sec:.0} ops/s)");
            assert!(
                ops_per_sec >= 10_000.0,
                "typing throughput {ops_per_sec:.0} ops/s < 10,000 threshold"
            );

            elapsed
        });
    });

    group.finish();
}

/// Benchmark: typing throughput on a large file (100k lines).
/// Simulates realistic editing scenario where user opens a large file and types.
fn bench_typing_throughput_large_file(c: &mut Criterion) {
    let mut group = c.benchmark_group("typing_large");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("insert_1k_chars_into_100k_lines", |b| {
        b.iter_custom(|_iters| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("large_typing.txt");
            let mut file = std::fs::File::create(&path).unwrap();
            for i in 0..100_000 {
                writeln!(file, "Line {i}: The quick brown fox jumps over the lazy dog.").unwrap();
            }
            file.flush().unwrap();

            let mut document = DocumentModel::from_file(&path).expect("load large typing fixture");
            // Move cursor to middle of file (direct offset, not repeated move_down)
            let middle = document.buffer_len() / 2;
            document.cursor_move_to_offset(middle);

            let start = Instant::now();
            for _ in 0..1_000 {
                document.insert_at_cursor(b"x");
            }
            let elapsed = start.elapsed();

            let ops_per_sec = 1_000.0 / elapsed.as_secs_f64();
            eprintln!(
                "[typing_large] 1k inserts into 100k-line file in {elapsed:?} ({ops_per_sec:.0} ops/s)"
            );

            elapsed
        });
    });

    group.finish();
}

/// Benchmark: single char insert + measurement recompute on large files.
/// Measures per-insert cost including line index rebuild (rebuild_line_index_from_tb).
fn bench_single_char_insert_recompute(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_insert_recompute");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    for &line_count in &[1_000, 10_000, 100_000] {
        let label = format!("{line_count}_lines");
        group.bench_function(&label, |b| {
            // Setup: create file with known line count
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("recompute.txt");
            let mut file = std::fs::File::create(&path).unwrap();
            for i in 0..line_count {
                writeln!(file, "Line {i}: padding content for realistic line length.").unwrap();
            }
            file.flush().unwrap();

            let mut document = DocumentModel::from_file(&path).expect("load recompute fixture");

            b.iter(|| {
                document.insert_at_cursor(b"a");
                std::hint::black_box(document.cursor_offset());
            });
        });
    }

    group.finish();
}

/// plans.md §7: select 1 MB + extract < 16 ms.
fn bench_select_1mb_redraw(c: &mut Criterion) {
    let mut group = c.benchmark_group("select_1mb");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("select_and_extract_1mb", |b| {
        // Pre-create a 1MB document (setup, not measured)
        const SELECTION_BYTES: usize = 1024 * 1024;
        let chunk = "abcdefghijklmnopqrstuvwxyz0123456789 ";
        let mut source = chunk.repeat(SELECTION_BYTES.div_ceil(chunk.len()));
        source.truncate(SELECTION_BYTES);
        let mut document = DocumentModel::from_lines(&[source]);

        b.iter(|| {
            document.select_all();
            let text = document.extract_selected_text();
            std::hint::black_box(&text);
            // Clear selection for next iteration
            document.clear_selection();
        });
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50);
    targets = bench_scroll_single_frame, bench_scroll_60s_60fps, bench_typing_throughput, bench_typing_throughput_large_file, bench_single_char_insert_recompute, bench_select_1mb_redraw
}
criterion_main!(benches);
