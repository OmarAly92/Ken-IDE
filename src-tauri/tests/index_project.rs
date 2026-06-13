use std::fs;

use terax_lib::modules::index::project::run_index;
use terax_lib::modules::index::store::IndexStore;

#[test]
fn run_index_builds_a_queryable_store_for_a_temp_project() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/service.ts"),
        "export function run() {}\nclass Engine {}\n",
    )
    .unwrap();
    fs::write(root.join("src/types.ts"), "interface Repo {}\ntype Id = string;\n").unwrap();
    fs::write(root.join("src/ignore.tsx"), "function jsx() {}\n").unwrap();

    let store = IndexStore::default();
    run_index(root, &store, |_done, _total| {});

    let status = store.status();
    assert_eq!(status.file_count, 2);
    assert_eq!(status.symbol_count, 4);

    let hits = store.query("Engine", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "Engine");
    assert!(hits[0].path.ends_with("service.ts"));

    assert!(store.query("jsx", 10).is_empty());
}
