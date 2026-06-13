use criterion::{black_box, criterion_group, criterion_main, Criterion};
use terax_lib::modules::fs::search::fuzzy_rank;

fn bench_fuzzy_rank(c: &mut Criterion) {
    let keys: Vec<String> = (0..10_000)
        .map(|i| format!("src/module_{i}/component_{i}.tsx"))
        .collect();
    let refs: Vec<&str> = keys.iter().map(String::as_str).collect();

    c.bench_function("fuzzy_rank/10k", |b| {
        b.iter(|| fuzzy_rank(black_box("comp"), black_box(&refs), 200))
    });
}

criterion_group!(benches, bench_fuzzy_rank);
criterion_main!(benches);
