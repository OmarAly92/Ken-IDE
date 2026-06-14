use criterion::{criterion_group, criterion_main, Criterion};

use terax_lib::modules::index::persist::{self, PersistedFile, PersistedIndex};
use terax_lib::modules::index::symbols::{Symbol, SymbolKind};

fn make_index(n: usize) -> PersistedIndex {
    let files = (0..n)
        .map(|i| PersistedFile {
            path: format!("/proj/file{i}.ts"),
            mtime_ms: i as u64,
            symbols: vec![Symbol {
                name: format!("fn_{i}"),
                kind: SymbolKind::Function,
                start_line: 1,
                end_line: 2,
            }],
        })
        .collect();
    PersistedIndex {
        version: 1,
        root: "/proj".to_string(),
        files,
    }
}

fn bench_save_load(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench.kenidx");
    let index = make_index(5000);
    persist::save(&path, &index).unwrap();

    c.bench_function("persist_load_5k", |b| {
        b.iter(|| {
            let loaded = persist::load(&path).unwrap();
            assert_eq!(loaded.files.len(), 5000);
        })
    });

    c.bench_function("persist_save_5k", |b| {
        b.iter(|| {
            persist::save(&path, &index).unwrap();
        })
    });
}

criterion_group!(benches, bench_save_load);
criterion_main!(benches);
