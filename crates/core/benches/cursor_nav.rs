use core::helpers::CoordType;
use core::types::VisualPoint;
use criterion::{Criterion, criterion_group, criterion_main};

fn make_ascii_1mb() -> Vec<u8> {
    let line = "The quick brown fox jumps over the lazy dog. ";
    let mut buf = Vec::with_capacity(1_048_576);
    while buf.len() < 1_048_576 {
        buf.extend_from_slice(line.as_bytes());
    }
    buf
}

fn bench_cursor_nav_walk_terminal_1mb(c: &mut Criterion) {
    let buf = make_ascii_1mb();
    c.bench_function("cursor_nav_walk_terminal_1mb", |b| {
        b.iter(|| {
            let slice = buf.as_slice();
            let mut cfg = core::unicode::CursorNav::new(&slice);
            let _ = cfg.goto_visual(VisualPoint { column: CoordType::MAX, row: usize::MAX });
        });
    });
}

fn bench_cursor_nav_walk_pixel_1mb(c: &mut Criterion) {
    let buf = make_ascii_1mb();
    let advance = core::unicode::PixelAdvance::default();
    c.bench_function("cursor_nav_walk_pixel_1mb", |b| {
        b.iter(|| {
            let slice = buf.as_slice();
            let mut cfg = core::unicode::CursorNav::with_advance(&slice, advance.clone());
            let _ = cfg.goto_visual(VisualPoint { column: CoordType::MAX, row: usize::MAX });
        });
    });
}

criterion_group!(benches, bench_cursor_nav_walk_terminal_1mb, bench_cursor_nav_walk_pixel_1mb);
criterion_main!(benches);
