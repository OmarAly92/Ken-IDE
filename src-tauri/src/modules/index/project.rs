use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::modules::fs::to_canon;
use crate::modules::index::store::IndexStore;
use crate::modules::index::symbols::extract_symbols;

const PRUNE_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
    ".venv",
    "__pycache__",
];

pub fn is_indexable(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("ts") | Some("mts") | Some("cts")
    )
}

pub fn collect_indexable_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|e| {
            !(e.file_type().is_some_and(|t| t.is_dir())
                && e
                    .file_name()
                    .to_str()
                    .is_some_and(|n| PRUNE_DIRS.contains(&n)))
        })
        .build();
    for entry in walker.flatten() {
        let path = entry.path();
        if path.is_file() && is_indexable(path) {
            out.push(path.to_path_buf());
        }
    }
    out
}

pub fn run_index(root: &Path, store: &IndexStore, mut on_progress: impl FnMut(usize, usize)) {
    let files = collect_indexable_files(root);
    let total = files.len();
    for (i, path) in files.iter().enumerate() {
        if let Ok(src) = std::fs::read_to_string(path) {
            store.replace_file(to_canon(path), extract_symbols(&src));
        }
        on_progress(i + 1, total);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn is_indexable_matches_only_typescript_non_jsx() {
        assert!(is_indexable(Path::new("a.ts")));
        assert!(is_indexable(Path::new("a.mts")));
        assert!(is_indexable(Path::new("a.cts")));
        assert!(!is_indexable(Path::new("a.tsx")));
        assert!(!is_indexable(Path::new("a.js")));
        assert!(!is_indexable(Path::new("a.json")));
        assert!(!is_indexable(Path::new("README")));
    }

    #[test]
    fn collect_skips_pruned_dirs_and_non_ts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.ts", "function a() {}");
        write(root, "src/b.tsx", "function b() {}");
        write(root, "src/c.js", "function c() {}");
        write(root, "node_modules/dep/d.ts", "function d() {}");
        write(root, "target/e.ts", "function e() {}");

        let mut names: Vec<String> = collect_indexable_files(root)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.ts".to_string()]);
    }

    #[test]
    fn run_index_populates_store_and_reports_progress() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.ts", "function greet() {}\nclass Svc {}\n");
        write(root, "sub/b.ts", "interface Repo {}\n");

        let store = IndexStore::default();
        let mut progress: Vec<(usize, usize)> = Vec::new();
        run_index(root, &store, |done, total| progress.push((done, total)));

        let status = store.status();
        assert_eq!(status.file_count, 2);
        assert_eq!(status.symbol_count, 3);
        assert_eq!(progress.len(), 2);
        assert_eq!(progress.last(), Some(&(2, 2)));
        assert_eq!(store.query("greet", 10).len(), 1);
        assert_eq!(store.query("Repo", 10).len(), 1);
    }
}
