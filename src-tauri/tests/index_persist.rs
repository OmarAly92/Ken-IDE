use std::fs;
use std::time::{Duration, Instant};

use terax_lib::modules::index::persist::{self, PersistedFile, PersistedIndex};
use terax_lib::modules::index::project::{file_mtime_ms, load_or_index, run_index};
use terax_lib::modules::index::store::IndexStore;
use terax_lib::modules::index::symbols::{Symbol, SymbolKind};

fn to_canon(p: &std::path::Path) -> String {
    terax_lib::modules::fs::to_canon(p)
}

#[test]
fn warm_reopen_loads_from_cache_and_reconciles() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a.ts"), "export function run() {}\n").unwrap();
    fs::write(root.join("src/b.ts"), "class Engine {}\n").unwrap();

    let cache = dir.path().join("cache").join("index.kenidx");

    let store1 = IndexStore::default();
    store1.set_root(Some(to_canon(root)));
    load_or_index(root, &cache, &store1, |_d, _t| {});
    assert!(cache.exists());
    assert_eq!(store1.status().file_count, 2);

    let store2 = IndexStore::default();
    store2.set_root(Some(to_canon(root)));
    load_or_index(root, &cache, &store2, |_d, _t| {});
    assert_eq!(store2.status().file_count, 2);
    assert_eq!(store2.query("Engine", 10).len(), 1);
    assert_eq!(store2.query("run", 10).len(), 1);
}

#[test]
fn cold_start_load_is_fast() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = dir.path().join("big.kenidx");

    let files: Vec<PersistedFile> = (0..3000)
        .map(|i| PersistedFile {
            path: format!("/proj/file{i}.ts"),
            mtime_ms: i as u64,
            symbols: vec![
                Symbol {
                    name: format!("fn_{i}"),
                    kind: SymbolKind::Function,
                    start_line: 1,
                    end_line: 2,
                },
                Symbol {
                    name: format!("Class_{i}"),
                    kind: SymbolKind::Class,
                    start_line: 3,
                    end_line: 4,
                },
            ],
        })
        .collect();
    let index = PersistedIndex {
        version: 1,
        root: "/proj".to_string(),
        files,
    };
    persist::save(&cache, &index).unwrap();

    let start = Instant::now();
    let loaded = persist::load(&cache).expect("load");
    let elapsed = start.elapsed();

    assert_eq!(loaded.files.len(), 3000);
    assert!(
        elapsed < Duration::from_millis(250),
        "cold-start load took {elapsed:?}, budget 250ms"
    );
}

#[test]
fn full_index_then_save_then_reload_equivalent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    fs::write(root.join("x.ts"), "function alpha() {}\ninterface Beta {}\n").unwrap();
    let cache = dir.path().join("x.kenidx");

    let store = IndexStore::default();
    store.set_root(Some(to_canon(root)));
    run_index(root, &store, |_d, _t| {});
    persist::save(&cache, &store.snapshot()).unwrap();

    let reloaded = persist::load(&cache).expect("load");
    let key = to_canon(&root.join("x.ts"));
    let entry = reloaded.files.iter().find(|f| f.path == key).expect("entry");
    assert_eq!(entry.mtime_ms, file_mtime_ms(&root.join("x.ts")).unwrap());
    assert_eq!(entry.symbols.len(), 2);
}
