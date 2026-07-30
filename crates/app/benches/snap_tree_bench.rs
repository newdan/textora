use criterion::{Criterion, criterion_group, criterion_main};
use textora_app::dev_support::{DisplayLineEntry, SnapTree};

fn splice_18000_entries(c: &mut Criterion) {
    let entries: Vec<_> =
        (0..18_000).map(|i| DisplayLineEntry::placeholder(i * 200, 200, i as u64, 1)).collect();
    c.bench_function("snap_tree/splice_18000_entries", |b| {
        b.iter_batched(
            || SnapTree::from_entries(entries.clone()),
            |mut tree| {
                let replacement = DisplayLineEntry::placeholder(1_000_000, 200, 9_999, 1);
                criterion::black_box(tree.splice(5_000..5_001, vec![replacement]));
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, splice_18000_entries);
criterion_main!(benches);
