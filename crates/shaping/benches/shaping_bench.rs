use criterion::{Criterion, criterion_group, criterion_main};
use shaping::Shaper;

fn bench_shape_ascii_1k_chars(c: &mut Criterion) {
    let mut shaper = Shaper::new().expect("shaper creation failed");
    let text: String = "The quick brown fox jumps over the lazy dog. ".repeat(23); // 45*23 = 1035 chars
    debug_assert!(text.len() >= 1000);

    c.bench_function("shape_ascii_1k_chars", |b| {
        b.iter(|| {
            let run = shaper.shape(&text).expect("shaping failed");
            std::hint::black_box(&run);
        });
    });
}

fn bench_grapheme_advance_lookup(c: &mut Criterion) {
    let mut shaper = Shaper::new().expect("shaper creation failed");

    // Warm up the cache: shape each grapheme once
    let graphemes = ["A", "B", "C", "D", "E", "a", "b", "c", "d", "e"];
    for g in &graphemes {
        let _ = shaper.grapheme_advance(g);
    }

    // Benchmark cache-hit path
    c.bench_function("grapheme_advance_lookup", |b| {
        b.iter(|| {
            for g in &graphemes {
                let advance = shaper.grapheme_advance(g).expect("advance failed");
                std::hint::black_box(advance);
            }
        });
    });
}

criterion_group!(benches, bench_shape_ascii_1k_chars, bench_grapheme_advance_lookup);
criterion_main!(benches);
