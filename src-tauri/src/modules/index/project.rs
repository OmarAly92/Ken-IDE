use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ignore::WalkBuilder;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::modules::fs::to_canon;
use crate::modules::index::persist::{self, PersistedIndex};
use crate::modules::index::store::{IndexStatus, IndexStore, SymbolHit};
use crate::modules::index::symbols::extract_symbols;
use crate::modules::workspace::WorkspaceRegistry;

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

pub fn file_mtime_ms(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let dur = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(dur.as_millis() as u64)
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
            store.replace_file(to_canon(path), file_mtime_ms(path).unwrap_or(0), extract_symbols(&src));
        }
        on_progress(i + 1, total);
    }
}

pub fn reconcile(
    root: &Path,
    loaded: PersistedIndex,
    store: &IndexStore,
    mut on_progress: impl FnMut(usize, usize),
) {
    let mut cached: HashMap<String, crate::modules::index::persist::PersistedFile> = loaded
        .files
        .into_iter()
        .map(|f| (f.path.clone(), f))
        .collect();
    let files = collect_indexable_files(root);
    let total = files.len();
    for (i, path) in files.iter().enumerate() {
        let key = to_canon(path);
        let current = file_mtime_ms(path).unwrap_or(u64::MAX);
        match cached.remove(&key) {
            Some(entry) if entry.mtime_ms >= current => {
                store.replace_file(key, entry.mtime_ms, entry.symbols);
            }
            _ => {
                if let Ok(src) = std::fs::read_to_string(path) {
                    store.replace_file(key, current, extract_symbols(&src));
                }
            }
        }
        on_progress(i + 1, total);
    }
}

pub fn load_or_index(
    root: &Path,
    cache_path: &Path,
    store: &IndexStore,
    on_progress: impl FnMut(usize, usize),
) {
    let canon = to_canon(root);
    match persist::load(cache_path) {
        Some(loaded) if loaded.root == canon => reconcile(root, loaded, store, on_progress),
        _ => run_index(root, store, on_progress),
    }
    let _ = persist::save(cache_path, &store.snapshot());
}

const DEBOUNCE: Duration = Duration::from_millis(150);
const MAX_WINDOW: Duration = Duration::from_millis(1000);

pub fn apply_change(store: &IndexStore, path: &Path) {
    if !is_indexable(path) {
        return;
    }
    let key = to_canon(path);
    match std::fs::read_to_string(path) {
        Ok(src) => store.replace_file(key, file_mtime_ms(path).unwrap_or(0), extract_symbols(&src)),
        Err(_) => store.remove_file(&key),
    }
}

#[derive(Default)]
pub struct IndexWatchState {
    inner: Mutex<Option<RecommendedWatcher>>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressPayload {
    indexed: usize,
    total: usize,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdatedPayload {
    paths: Vec<String>,
}

fn collect(set: &mut HashSet<PathBuf>, ev: notify::Result<Event>) {
    let Ok(ev) = ev else { return };
    if matches!(ev.kind, EventKind::Access(_)) {
        return;
    }
    for p in ev.paths {
        set.insert(p);
    }
}

fn watch_drain(rx: mpsc::Receiver<notify::Result<Event>>, app: AppHandle) {
    loop {
        let first = match rx.recv() {
            Ok(ev) => ev,
            Err(_) => return,
        };
        let mut paths: HashSet<PathBuf> = HashSet::new();
        collect(&mut paths, first);

        let deadline = Instant::now() + MAX_WINDOW;
        loop {
            let timeout = DEBOUNCE.min(deadline.saturating_duration_since(Instant::now()));
            match rx.recv_timeout(timeout) {
                Ok(ev) => collect(&mut paths, ev),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if Instant::now() >= deadline {
                break;
            }
        }

        let store = app.state::<IndexStore>();
        let mut changed: Vec<String> = Vec::new();
        for path in paths {
            if is_indexable(&path) {
                apply_change(&store, &path);
                changed.push(to_canon(&path));
            }
        }
        if !changed.is_empty() {
            let _ = app.emit("index:updated", UpdatedPayload { paths: changed });
        }
    }
}

fn start_watch(state: &IndexWatchState, app: &AppHandle, root: &Path) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default(),
    )
    .map_err(|e| e.to_string())?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;

    let app_for_thread = app.clone();
    std::thread::Builder::new()
        .name("ken-index-watch".into())
        .spawn(move || watch_drain(rx, app_for_thread))
        .map_err(|e| e.to_string())?;

    *state.inner.lock().expect("index watch poisoned") = Some(watcher);
    Ok(())
}

#[tauri::command]
pub fn index_project(
    root: String,
    app: AppHandle,
    store: State<'_, IndexStore>,
    watch: State<'_, IndexWatchState>,
    registry: State<'_, WorkspaceRegistry>,
) -> Result<(), String> {
    let root_path = registry.authorize(&root).map_err(|e| e.to_string())?;
    if !root_path.is_dir() {
        return Err(format!("not a directory: {root}"));
    }
    store.clear();
    store.set_root(Some(to_canon(&root_path)));
    start_watch(&watch, &app, &root_path)?;

    let app_for_thread = app.clone();
    std::thread::Builder::new()
        .name("ken-index-build".into())
        .spawn(move || {
            let store = app_for_thread.state::<IndexStore>();
            let mut last_emit = 0usize;
            run_index(&root_path, &store, |indexed, total| {
                if indexed == total || indexed - last_emit >= 50 {
                    last_emit = indexed;
                    let _ = app_for_thread
                        .emit("index:progress", ProgressPayload { indexed, total });
                }
            });
            let status = store.status();
            let _ = app_for_thread.emit("index:done", status);
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn query_symbols(
    query: String,
    limit: Option<usize>,
    store: State<'_, IndexStore>,
) -> Vec<SymbolHit> {
    store.query(&query, limit.unwrap_or(50).min(500))
}

#[tauri::command]
pub fn index_status(store: State<'_, IndexStore>) -> IndexStatus {
    store.status()
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

    #[test]
    fn reconcile_keeps_unchanged_reparses_changed_adds_new_drops_deleted() {
        use crate::modules::index::persist::{PersistedFile, PersistedIndex};
        use crate::modules::index::symbols::{Symbol, SymbolKind};

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "keep.ts", "function realKeep() {}\n");
        write(root, "change.ts", "function before() {}\n");
        write(root, "new.ts", "function fresh() {}\n");

        let keep_mtime = file_mtime_ms(&root.join("keep.ts")).unwrap();

        let loaded = PersistedIndex {
            version: 1,
            root: to_canon(root),
            files: vec![
                PersistedFile {
                    path: to_canon(&root.join("keep.ts")),
                    mtime_ms: keep_mtime,
                    symbols: vec![Symbol {
                        name: "SENTINEL_FROM_CACHE".to_string(),
                        kind: SymbolKind::Function,
                        start_line: 1,
                        end_line: 1,
                    }],
                },
                PersistedFile {
                    path: to_canon(&root.join("change.ts")),
                    mtime_ms: 0,
                    symbols: vec![Symbol {
                        name: "before".to_string(),
                        kind: SymbolKind::Function,
                        start_line: 1,
                        end_line: 1,
                    }],
                },
                PersistedFile {
                    path: to_canon(&root.join("gone.ts")),
                    mtime_ms: 0,
                    symbols: vec![Symbol {
                        name: "deleted".to_string(),
                        kind: SymbolKind::Function,
                        start_line: 1,
                        end_line: 1,
                    }],
                },
            ],
        };

        write(root, "change.ts", "function after() {}\n");

        let store = IndexStore::default();
        reconcile(root, loaded, &store, |_done, _total| {});

        assert_eq!(store.query("SENTINEL_FROM_CACHE", 10).len(), 1);
        assert!(store.query("realKeep", 10).is_empty());
        assert_eq!(store.query("after", 10).len(), 1);
        assert!(store.query("before", 10).is_empty());
        assert_eq!(store.query("fresh", 10).len(), 1);
        assert!(store.query("deleted", 10).is_empty());
        assert_eq!(store.status().file_count, 3);
    }

    #[test]
    fn apply_change_upserts_and_removes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("a.ts");
        fs::write(&file, "function one() {}\n").unwrap();

        let store = IndexStore::default();
        apply_change(&store, &file);
        assert_eq!(store.query("one", 10).len(), 1);

        fs::write(&file, "function two() {}\n").unwrap();
        apply_change(&store, &file);
        assert!(store.query("one", 10).is_empty());
        assert_eq!(store.query("two", 10).len(), 1);

        fs::remove_file(&file).unwrap();
        apply_change(&store, &file);
        assert!(store.query("two", 10).is_empty());

        let ignored = root.join("note.txt");
        fs::write(&ignored, "hello").unwrap();
        let before = store.status().file_count;
        apply_change(&store, &ignored);
        assert_eq!(store.status().file_count, before);
    }
}
