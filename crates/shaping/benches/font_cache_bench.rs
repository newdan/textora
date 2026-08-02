use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use criterion::{Criterion, criterion_group, criterion_main};

struct FontCacheFixture {
    path: PathBuf,
}

impl FontCacheFixture {
    fn unique() -> Self {
        let sequence = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock must be after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir()
            .join(format!("textora-font-cache-bench-{}-{sequence}.bin", std::process::id()));
        Self { path }
    }
}

impl Drop for FontCacheFixture {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("could not remove font cache benchmark fixture: {error}");
        }
    }
}

fn benchmark_font_cache(criterion: &mut Criterion) {
    let cache_fixture = FontCacheFixture::unique();
    let cache_miss_started_at = Instant::now();
    let font_system = shaping::font_cache::new_font_system_with_cache(&cache_fixture.path);
    std::hint::black_box(font_system);
    eprintln!(
        "[font-cache-bench] cache_miss={:.2}ms",
        cache_miss_started_at.elapsed().as_secs_f64() * 1_000.0
    );

    criterion.bench_function("startup/font_cache_hit", |bencher| {
        bencher.iter(|| {
            std::hint::black_box(shaping::font_cache::new_font_system_with_cache(
                &cache_fixture.path,
            ));
        });
    });
}

criterion_group!(benches, benchmark_font_cache);
criterion_main!(benches);
