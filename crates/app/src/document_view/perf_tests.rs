use super::*;

use std::io::Write;
use std::time::Instant;

/// plans.md §6: single char insert + rebuild_line_index_from_tb < 1ms on 10k lines.
#[test]
fn single_insert_threshold_10k_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("threshold.txt");
    let mut file = std::fs::File::create(&path).unwrap();
    for i in 0..10_000 {
        writeln!(file, "Line {i}: padding content for realistic line length.").unwrap();
    }
    file.flush().unwrap();

    let mut dv = DocumentView::from_file(&path, 50, 10.0).expect("load failed");

    // Warm up
    dv.insert_at_cursor(b"a");
    std::hint::black_box(dv.cursor().offset);

    // Measure 100 iterations and take the average
    let iterations = 100;
    let start = Instant::now();
    for _ in 0..iterations {
        dv.insert_at_cursor(b"a");
        std::hint::black_box(dv.cursor().offset);
    }
    let elapsed = start.elapsed();
    let per_op = elapsed / iterations;

    assert!(
        per_op.as_millis() < 1,
        "single char insert + recompute took {per_op:?} per op (threshold: < 1ms, 10k lines)",
    );
}
