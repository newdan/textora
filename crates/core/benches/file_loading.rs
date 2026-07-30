use criterion::{Criterion, criterion_group, criterion_main};
use std::io::Write;
use std::time::{Duration, Instant};

fn generate_ascii_file(path: &std::path::Path, size_mb: usize) {
    let line =
        "The quick brown fox jumps over the lazy dog. This is a test line for benchmarking.\n";
    let target = size_mb * 1024 * 1024;
    let mut file = std::fs::File::create(path).unwrap();
    let mut written = 0;
    while written < target {
        let chunk = &line.as_bytes()[..(target - written).min(line.len())];
        file.write_all(chunk).unwrap();
        written += chunk.len();
    }
    file.flush().unwrap();
}

fn generate_cjk_file(path: &std::path::Path, size_mb: usize) {
    let line =
        "这是一行中日韩混合文本，用于基准测试。CJK mixed text for benchmarking. 世界你好。\n";
    let target = size_mb * 1024 * 1024;
    let mut file = std::fs::File::create(path).unwrap();
    let mut written = 0;
    while written < target {
        let remaining = target - written;
        if remaining >= line.len() {
            file.write_all(line.as_bytes()).unwrap();
            written += line.len();
        } else {
            file.write_all(&line.as_bytes()[..remaining]).unwrap();
            written += remaining;
        }
    }
    file.flush().unwrap();
}

fn generate_dense_cjk_file(path: &std::path::Path, size_mb: usize) {
    let line = "你好世界春夏秋冬山河湖海天地玄黄宇宙洪荒\n";
    let line_len = line.len();
    // Round target down to nearest multiple of line_len so the file
    // is valid UTF-8 end-to-end (no truncated multi-byte sequences at EOF).
    let target = ((size_mb * 1024 * 1024) / line_len) * line_len;
    let mut file = std::fs::File::create(path).expect("create dense CJK benchmark file");
    let mut written = 0;

    while written < target {
        file.write_all(line.as_bytes()).expect("write dense CJK benchmark line");
        written += line_len;
    }

    file.flush().expect("flush dense CJK benchmark file");
}

fn bench_open_50mb_ascii(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench_50mb_ascii.txt");
    generate_ascii_file(&path, 50);

    c.bench_function("open_50mb_ascii", |b| {
        b.iter(|| {
            let (buf, meta) = core::file::load_file(&path).expect("load failed");
            std::hint::black_box((&buf, &meta));
        });
    });
}

fn bench_open_50mb_cjk(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench_50mb_cjk.txt");
    generate_cjk_file(&path, 50);

    c.bench_function("open_50mb_cjk", |b| {
        b.iter(|| {
            let (buf, meta) = core::file::load_file(&path).expect("load failed");
            std::hint::black_box((&buf, &meta));
        });
    });
}

fn bench_open_50mb_dense_cjk(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("create benchmark tempdir");
    let path = dir.path().join("bench_50mb_dense_cjk.txt");
    generate_dense_cjk_file(&path, 50);

    c.bench_function("open_50mb_dense_cjk", |b| {
        b.iter(|| {
            let (buf, meta) = core::file::load_file(&path).expect("load dense CJK benchmark file");
            assert_eq!(meta.original_encoding, None);
            std::hint::black_box((&buf, &meta));
        });
    });
}

fn bench_open_200mb_ascii(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench_200mb_ascii.txt");
    generate_ascii_file(&path, 200);

    c.bench_function("open_200mb_ascii", |b| {
        b.iter(|| {
            let (buf, meta) = core::file::load_file(&path).expect("load failed");
            std::hint::black_box((&buf, &meta));
        });
    });
}

/// Threshold validation: plans.md §5 performance gates.
/// Runs each load 10 times, reports P95, asserts against threshold.
fn validate_thresholds(c: &mut Criterion) {
    // ASCII 50 MB — P95 < 80ms
    {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("threshold_50mb_ascii.txt");
        generate_ascii_file(&path, 50);

        c.bench_function("threshold_open_50mb_ascii", |b| {
            b.iter_custom(|_iters| {
                let mut times = Vec::with_capacity(10);
                for _ in 0..10 {
                    let start = Instant::now();
                    let (buf, _meta) = core::file::load_file(&path).expect("load failed");
                    times.push(start.elapsed());
                    std::hint::black_box(&buf);
                }
                times.sort();
                let p95 = times[7]; // 8th of 10 (0-indexed)
                eprintln!("[threshold] 50mb_ascii P95 = {p95:?}");
                assert!(p95 < Duration::from_millis(80), "50mb_ascii P95 {p95:?} >= 80ms");
                times.iter().sum::<Duration>()
            });
        });
    }

    // CJK 50 MB — P95 < 200ms
    {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("threshold_50mb_cjk.txt");
        generate_cjk_file(&path, 50);

        c.bench_function("threshold_open_50mb_cjk", |b| {
            b.iter_custom(|_iters| {
                let mut times = Vec::with_capacity(10);
                for _ in 0..10 {
                    let start = Instant::now();
                    let (buf, _meta) = core::file::load_file(&path).expect("load failed");
                    times.push(start.elapsed());
                    std::hint::black_box(&buf);
                }
                times.sort();
                let p95 = times[7];
                eprintln!("[threshold] 50mb_cjk P95 = {p95:?}");
                assert!(p95 < Duration::from_millis(200), "50mb_cjk P95 {p95:?} >= 200ms");
                times.iter().sum::<Duration>()
            });
        });
    }

    // ASCII 200 MB — P95 < 400ms
    {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("threshold_200mb_ascii.txt");
        generate_ascii_file(&path, 200);

        c.bench_function("threshold_open_200mb_ascii", |b| {
            b.iter_custom(|_iters| {
                let mut times = Vec::with_capacity(10);
                for _ in 0..10 {
                    let start = Instant::now();
                    let (buf, _meta) = core::file::load_file(&path).expect("load failed");
                    times.push(start.elapsed());
                    std::hint::black_box(&buf);
                }
                times.sort();
                let p95 = times[7];
                eprintln!("[threshold] 200mb_ascii P95 = {p95:?}");
                assert!(p95 < Duration::from_millis(400), "200mb_ascii P95 {p95:?} >= 400ms");
                times.iter().sum::<Duration>()
            });
        });
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench_open_50mb_ascii, bench_open_50mb_cjk, bench_open_50mb_dense_cjk, bench_open_200mb_ascii, validate_thresholds
}
criterion_main!(benches);
