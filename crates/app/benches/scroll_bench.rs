use criterion::{Criterion, criterion_group, criterion_main};
use std::io::Write;
use std::time::{Duration, Instant};
use textora_app::dev_support::DocumentView;

const BENCH_LINE_HEIGHT: f32 = 24.27;

fn scroll_one_doc_line(dv: &mut DocumentView) {
    dv.scroll_doc_lines_for_viewport(1, BENCH_LINE_HEIGHT);
}

fn bench_scroll_single_frame(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scroll_frame.txt");
    let mut file = std::fs::File::create(&path).unwrap();
    for i in 0..100_000 {
        writeln!(file, "Line {i}: The quick brown fox jumps over the lazy dog.").unwrap();
    }
    file.flush().unwrap();

    let mut dv = DocumentView::from_file(&path, 50, 10.0).expect("load failed");

    c.bench_function("scroll_single_frame", |b| {
        b.iter(|| {
            scroll_one_doc_line(&mut dv);
            let vis = dv.visible_lines_with_line_height(BENCH_LINE_HEIGHT);
            std::hint::black_box(&vis);
            if dv.viewport_anchor_doc_line() > 99_950 {
                dv.scroll_to_doc_line_for_viewport(0, BENCH_LINE_HEIGHT);
            }
        });
    });
}

/// Simulate 60 seconds of continuous scrolling at 60 fps.
/// Reports frame time histogram and asserts < 1% dropped frames.
fn bench_scroll_60s_60fps(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scroll_60s.txt");
    let mut file = std::fs::File::create(&path).unwrap();
    for i in 0..100_000 {
        writeln!(file, "Line {i}: The quick brown fox jumps over the lazy dog. Extra padding to simulate realistic line length around 80 chars.").unwrap();
    }
    file.flush().unwrap();

    // Use sample_size=5 to keep total runtime reasonable (5 × ~1s each)
    let mut group = c.benchmark_group("scroll_60s");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("scroll_3600_frames", |b| {
        b.iter_custom(|_iters| {
            let mut dv = DocumentView::from_file(&path, 50, 10.0).expect("load failed");

            let total_frames = 3600u64; // 60s × 60fps
            let frame_budget = Duration::from_nanos(16_666_666); // ~16.67ms
            let mut dropped = 0u64;
            let mut total_time = Duration::ZERO;

            for _ in 0..total_frames {
                let frame_start = Instant::now();

                scroll_one_doc_line(&mut dv);
                let vis = dv.visible_lines_with_line_height(BENCH_LINE_HEIGHT);
                std::hint::black_box(&vis);

                let frame_time = frame_start.elapsed();
                total_time += frame_time;
                if frame_time > frame_budget {
                    dropped += 1;
                }

                if dv.viewport_anchor_doc_line() > 99_950 {
                    dv.scroll_to_doc_line_for_viewport(0, BENCH_LINE_HEIGHT);
                }
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

            let mut dv = DocumentView::from_file(&path, 50, 10.0).expect("load failed");
            let char_data = "a".as_bytes();

            let start = Instant::now();
            for _ in 0..10_000 {
                dv.insert_at_cursor(char_data);
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

            let mut dv = DocumentView::from_file(&path, 50, 10.0)
                .expect("load failed");
            // Move cursor to middle of file (direct offset, not repeated move_down)
            let mid = dv.buffer_len() / 2;
            dv.cursor_move_to_offset(mid);
            let char_data = "x".as_bytes();

            let start = Instant::now();
            for _ in 0..1_000 {
                dv.insert_at_cursor(char_data);
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

            let mut dv = DocumentView::from_file(&path, 50, 10.0).expect("load failed");

            b.iter(|| {
                dv.insert_at_cursor(b"a");
                std::hint::black_box(dv.cursor_offset());
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
        let mut dv = DocumentView::new(vec!["".to_string()], 50, 10.0);
        let chunk = b"abcdefghijklmnopqrstuvwxyz0123456789 ";
        let target = 1024 * 1024;
        while dv.buffer_len() < target {
            let remaining = target - dv.buffer_len();
            let to_write = &chunk[..chunk.len().min(remaining)];
            dv.insert_at_cursor(to_write);
        }

        b.iter(|| {
            dv.select_all();
            let text = dv.extract_selected_text();
            std::hint::black_box(&text);
            // Clear selection for next iteration
            dv.clear_selection();
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
