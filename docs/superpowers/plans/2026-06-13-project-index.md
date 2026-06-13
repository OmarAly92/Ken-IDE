# Project Index + Background Indexing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On project open, walk the workspace in a background thread, parse every TypeScript file into symbols (reusing Plan 3's `extract_symbols`), hold them in an in-memory store, keep that store live as files change, and let the frontend watch indexing progress and fuzzy-query any symbol project-wide.

**Architecture:** A new `IndexStore` (Tauri-managed `State`, interior-`Mutex` over a `BTreeMap<path, Vec<Symbol>>`) is the single source of truth for project symbols. A pure `run_index(root, store, on_progress)` collects indexable files with `ignore::WalkBuilder` and populates the store. The `index_project(root)` command clears the store, spawns a background thread that runs `run_index` while emitting `index:progress`/`index:done` events, and starts a dedicated **recursive** `notify` watcher (the existing `fs/watch.rs` watcher is refcount-based + NonRecursive, scoped to UI-expanded dirs, so it cannot cover a whole project) that re-parses changed files via `apply_change` and emits `index:updated`. Querying reuses Plan 1's `fuzzy_rank`. The frontend gets hand-typed wrappers, a Zustand progress store, a `useProjectIndex(root)` hook, and a status-bar indicator.

**Tech Stack:** Rust, `tree-sitter` (via Plan 3's `extract_symbols`), `ignore::WalkBuilder`, `notify` (recursive watcher), `nucleo-matcher` (via `fuzzy_rank`), `serde`, Tauri commands + events, TypeScript + React + Zustand + `@tauri-apps/api`.

**Scope discipline (locked decisions):**
- **No `tauri-specta`.** Hand-typed `invoke<T>(...)` + `listen<T>(...)`, matching the rest of the codebase and Plan 3.
- **`.ts` / `.mts` / `.cts` only.** Single `LANGUAGE_TYPESCRIPT` grammar (the `.tsx` extension is NOT matched). `.tsx`/`.jsx` and per-extension grammar dispatch are deferred.
- **Symbols + files only — no graph edges.** Imports/exports/dependency edges and dead-code are Plan 7. Routes/components are later.
- **In-memory only — no persistence.** On-disk `rkyv`/`memmap2` persistence is Plan 5; every `index_project` call rebuilds from scratch.
- **Full index on open AND live incremental on change** (both, per the user's scope decision). Incremental uses the index's own recursive watcher.
- **Sync core + std threads — no `tokio`.** Indexing is CPU-bound; mirrors `fs/watch.rs`'s `std::thread` + `mpsc` debounce model.
- **Wire field casing = camelCase** via `#[serde(rename_all = "camelCase")]` on every emitted struct (the repo-wide convention; the lone snake-case slip in Plan 3 was corrected).

---

## File Structure

| File | Responsibility |
|---|---|
| `src-tauri/src/modules/index/mod.rs` | Register `pub mod store;` and `pub mod project;` (alongside existing `pub mod symbols;`) |
| `src-tauri/src/modules/index/store.rs` | `IndexStore` (managed State), `SymbolHit`, `IndexStatus`; pure store ops + `query`; unit tests |
| `src-tauri/src/modules/index/project.rs` | `is_indexable`, `collect_indexable_files`, `run_index`, `apply_change`, the recursive `IndexWatchState` watcher, and the `index_project` / `query_symbols` / `index_status` commands; unit tests |
| `src-tauri/src/lib.rs` | `.manage(...)` the two index states; register the three commands in `generate_handler!` |
| `src-tauri/tests/index_project.rs` | Integration test: temp project → `run_index` → assert store contents + query |
| `src/modules/index/project.ts` | Typed `indexProject`/`querySymbols`/`indexStatus` + `listenIndexProgress`/`listenIndexUpdated`/`listenIndexDone` + payload types |
| `src/modules/index/store.ts` | Zustand `useIndexStore` (phase + counters) |
| `src/modules/index/useProjectIndex.ts` | Hook: trigger `indexProject` on root change, subscribe to events, drive the store |
| `src/modules/index/IndexStatusItem.tsx` | Status-bar indicator reading the store |
| `src/modules/index/index.ts` | Barrel re-exports |
| `src/modules/statusbar/StatusBar.tsx` | Mount `useProjectIndex(cwd)` + render `<IndexStatusItem />` |

**Event names (new, colon-style to match the existing `fs:changed`):** `index:progress`, `index:done`, `index:updated`.

---

### Task 1: The in-memory `IndexStore` (pure, TDD)

**Files:**
- Create: `src-tauri/src/modules/index/store.rs`
- Modify: `src-tauri/src/modules/index/mod.rs`

- [ ] **Step 1: Register the module.** In `src-tauri/src/modules/index/mod.rs`, add `pub mod store;` below the existing `pub mod symbols;` so the file reads:
```rust
pub mod project;
pub mod store;
pub mod symbols;
```
(`project` is created in Task 2 — declaring it now means Task 1 will not compile standalone, so create an empty placeholder to keep the tree buildable: also create `src-tauri/src/modules/index/project.rs` containing exactly the single line `` (empty file) for now. Task 2 fills it. If you prefer to compile Task 1 in isolation, temporarily comment `pub mod project;` and uncomment it in Task 2 — but the empty-file approach is cleaner.)

- [ ] **Step 2: Write the failing tests.** Create `src-tauri/src/modules/index/store.rs`:
```rust
use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Serialize;

use crate::modules::fs::search::fuzzy_rank;
use crate::modules::index::symbols::{Symbol, SymbolKind};

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SymbolHit {
    pub name: String,
    pub kind: SymbolKind,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub root: Option<String>,
    pub file_count: usize,
    pub symbol_count: usize,
}

#[derive(Default)]
struct IndexData {
    root: Option<String>,
    files: BTreeMap<String, Vec<Symbol>>,
}

#[derive(Default)]
pub struct IndexStore {
    inner: Mutex<IndexData>,
}

impl IndexStore {
    pub fn clear(&self) {
        let mut data = self.inner.lock().expect("index store poisoned");
        data.root = None;
        data.files.clear();
    }

    pub fn set_root(&self, root: Option<String>) {
        self.inner.lock().expect("index store poisoned").root = root;
    }

    pub fn replace_file(&self, path: String, symbols: Vec<Symbol>) {
        self.inner
            .lock()
            .expect("index store poisoned")
            .files
            .insert(path, symbols);
    }

    pub fn remove_file(&self, path: &str) {
        self.inner
            .lock()
            .expect("index store poisoned")
            .files
            .remove(path);
    }

    pub fn status(&self) -> IndexStatus {
        let data = self.inner.lock().expect("index store poisoned");
        IndexStatus {
            root: data.root.clone(),
            file_count: data.files.len(),
            symbol_count: data.files.values().map(|v| v.len()).sum(),
        }
    }

    pub fn query(&self, query: &str, limit: usize) -> Vec<SymbolHit> {
        let data = self.inner.lock().expect("index store poisoned");
        let mut entries: Vec<(&str, &Symbol)> = Vec::new();
        for (path, syms) in data.files.iter() {
            for s in syms {
                entries.push((path.as_str(), s));
            }
        }
        let make = |(path, s): (&str, &Symbol)| SymbolHit {
            name: s.name.clone(),
            kind: s.kind.clone(),
            path: path.to_string(),
            start_line: s.start_line,
            end_line: s.end_line,
        };
        if query.trim().is_empty() {
            return entries.into_iter().take(limit).map(make).collect();
        }
        let names: Vec<&str> = entries.iter().map(|(_, s)| s.name.as_str()).collect();
        fuzzy_rank(query, &names, limit)
            .into_iter()
            .map(|i| make(entries[i]))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(name: &str, line: usize) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            start_line: line,
            end_line: line,
        }
    }

    #[test]
    fn empty_store_reports_zero_and_returns_no_hits() {
        let store = IndexStore::default();
        let status = store.status();
        assert_eq!(status.file_count, 0);
        assert_eq!(status.symbol_count, 0);
        assert_eq!(status.root, None);
        assert!(store.query("anything", 10).is_empty());
    }

    #[test]
    fn replace_file_then_query_finds_symbol() {
        let store = IndexStore::default();
        store.replace_file("a.ts".to_string(), vec![sym("greet", 1), sym("run", 5)]);
        let hits = store.query("greet", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "greet");
        assert_eq!(hits[0].path, "a.ts");
        assert_eq!(hits[0].start_line, 1);
        let status = store.status();
        assert_eq!(status.file_count, 1);
        assert_eq!(status.symbol_count, 2);
    }

    #[test]
    fn replace_file_overwrites_previous_symbols_for_that_path() {
        let store = IndexStore::default();
        store.replace_file("a.ts".to_string(), vec![sym("old", 1)]);
        store.replace_file("a.ts".to_string(), vec![sym("new", 2)]);
        assert_eq!(store.status().symbol_count, 1);
        assert!(store.query("old", 10).is_empty());
        assert_eq!(store.query("new", 10).len(), 1);
    }

    #[test]
    fn remove_file_drops_its_symbols() {
        let store = IndexStore::default();
        store.replace_file("a.ts".to_string(), vec![sym("greet", 1)]);
        store.remove_file("a.ts");
        assert_eq!(store.status().file_count, 0);
        assert!(store.query("greet", 10).is_empty());
    }

    #[test]
    fn query_respects_limit() {
        let store = IndexStore::default();
        store.replace_file(
            "a.ts".to_string(),
            vec![sym("handle", 1), sym("handler", 2), sym("handlers", 3)],
        );
        assert_eq!(store.query("handle", 2).len(), 2);
    }

    #[test]
    fn clear_resets_root_and_files() {
        let store = IndexStore::default();
        store.set_root(Some("/proj".to_string()));
        store.replace_file("a.ts".to_string(), vec![sym("greet", 1)]);
        store.clear();
        let status = store.status();
        assert_eq!(status.root, None);
        assert_eq!(status.file_count, 0);
    }
}
```

- [ ] **Step 3: Run the tests — they should pass immediately.** The implementation above is complete (this task is structured so the store and its tests land together; the "failing first" gate is satisfied by the next sub-step verifying that removing the impl breaks them — skip if you trust the suite).

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib index::store`
Expected: 6 tests pass. If `fuzzy_rank` is not `pub` in `crate::modules::fs::search`, confirm its visibility (Plan 1 made it `pub fn fuzzy_rank(query: &str, keys: &[&str], cap: usize) -> Vec<usize>`); it is imported here as `use crate::modules::fs::search::fuzzy_rank;`. If the path differs, fix the `use` and report it.

- [ ] **Step 4: Commit**
```bash
git add src-tauri/src/modules/index/store.rs src-tauri/src/modules/index/mod.rs src-tauri/src/modules/index/project.rs
git commit -m "feat(index): in-memory IndexStore with fuzzy symbol query"
```

---

### Task 2: File walking + the synchronous index core (TDD)

**Files:**
- Modify: `src-tauri/src/modules/index/project.rs` (currently empty from Task 1)

- [ ] **Step 1: Write the failing tests.** Replace the contents of `src-tauri/src/modules/index/project.rs` with:
```rust
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
        assert_eq!(progress.last(), Some(&(2, 2)));
        assert_eq!(store.query("greet", 10).len(), 1);
        assert_eq!(store.query("Repo", 10).len(), 1);
    }
}
```

- [ ] **Step 2: Run the tests**
Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib index::project`
Expected: 3 tests pass. If `ignore::WalkBuilder`'s `filter_entry` closure type or `flatten()` differs in this version, adapt minimally (the same crate is already used in `fs/search.rs` — cross-check that file's walker construction) and report any deviation.

- [ ] **Step 3: Commit**
```bash
git add src-tauri/src/modules/index/project.rs
git commit -m "feat(index): walk project and build symbol index synchronously"
```

---

### Task 3: Incremental `apply_change` + the recursive watcher + the `index_project` command

**Files:**
- Modify: `src-tauri/src/modules/index/project.rs`

- [ ] **Step 1: Write the failing test for `apply_change`.** Add this test inside the existing `mod tests` block in `src-tauri/src/modules/index/project.rs`:
```rust
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
```

- [ ] **Step 2: Run it to confirm it fails**
Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib index::project::tests::apply_change_upserts_and_removes`
Expected: FAIL — `apply_change` does not exist yet (compile error).

- [ ] **Step 3: Implement `apply_change`, the watcher, and the command.** Add the following to `src-tauri/src/modules/index/project.rs`. First extend the imports at the top of the file to:
```rust
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ignore::WalkBuilder;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::modules::fs::to_canon;
use crate::modules::index::store::{IndexStatus, IndexStore, SymbolHit};
use crate::modules::index::symbols::extract_symbols;
use crate::modules::workspace::WorkspaceRegistry;
```
Then add (after `run_index`, before the `#[cfg(test)]` block):
```rust
const DEBOUNCE: Duration = Duration::from_millis(150);
const MAX_WINDOW: Duration = Duration::from_millis(1000);

pub fn apply_change(store: &IndexStore, path: &Path) {
    if !is_indexable(path) {
        return;
    }
    let key = to_canon(path);
    match std::fs::read_to_string(path) {
        Ok(src) => store.replace_file(key, extract_symbols(&src)),
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
```

- [ ] **Step 4: Run the `apply_change` test, confirm it passes**
Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib index::project::tests::apply_change_upserts_and_removes`
Expected: PASS.

- [ ] **Step 5: Build the whole crate to confirm the commands + Tauri wiring compile**
Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: compiles. (`index_project`/`query_symbols`/`index_status` are not yet registered — that is Task 4 — but they must compile now. If `registry.authorize` has a different signature than `fn authorize<P: AsRef<Path>>(&self, path: P) -> std::io::Result<PathBuf>`, check `src-tauri/src/modules/workspace.rs` and adapt; report the actual signature.)

- [ ] **Step 6: Commit**
```bash
git add src-tauri/src/modules/index/project.rs
git commit -m "feat(index): recursive watcher, incremental apply_change, and index_project command"
```

---

### Task 4: Register the states + commands; integration test

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Create: `src-tauri/tests/index_project.rs`

- [ ] **Step 1: Register the managed states.** In `src-tauri/src/lib.rs`, in the builder chain alongside the other `.manage(...)` calls (e.g. right after `.manage(fs::grep::ContentSearchState::default())`), add:
```rust
        .manage(index::store::IndexStore::default())
        .manage(index::project::IndexWatchState::default())
```

- [ ] **Step 2: Register the three commands.** In the `tauri::generate_handler![ ... ]` list, after the existing `index::symbols::index_file_symbols,` line, add:
```rust
            index::project::index_project,
            index::project::query_symbols,
            index::project::index_status,
```

- [ ] **Step 3: Write the integration test.** Create `src-tauri/tests/index_project.rs`:
```rust
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
```

- [ ] **Step 4: Run the full Rust test suite**
Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS — the new integration test plus all `index::store` / `index::project` unit tests, no regressions. If the integration test cannot reach `run_index`/`IndexStore`, confirm `pub mod project;`/`pub mod store;` in `modules/index/mod.rs` and `pub mod modules;` in `lib.rs` (both are public from Task 1 / Plan 3); match how `src-tauri/tests/index_symbols.rs` imports lib internals.

- [ ] **Step 5: Commit**
```bash
git add src-tauri/src/lib.rs src-tauri/tests/index_project.rs
git commit -m "feat(index): register index states and project commands"
```

---

### Task 5: Frontend wrappers, progress store, hook, and status-bar indicator

**Files:**
- Create: `src/modules/index/project.ts`
- Create: `src/modules/index/store.ts`
- Create: `src/modules/index/useProjectIndex.ts`
- Create: `src/modules/index/IndexStatusItem.tsx`
- Modify: `src/modules/index/index.ts`
- Modify: `src/modules/statusbar/StatusBar.tsx`

- [ ] **Step 1: Create the typed command/event wrappers `src/modules/index/project.ts`.**
```ts
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { SymbolKind } from "./symbols";

export interface SymbolHit {
  name: string;
  kind: SymbolKind;
  path: string;
  startLine: number;
  endLine: number;
}

export interface IndexStatus {
  root: string | null;
  fileCount: number;
  symbolCount: number;
}

export interface IndexProgress {
  indexed: number;
  total: number;
}

export function indexProject(root: string): Promise<void> {
  return invoke<void>("index_project", { root });
}

export function querySymbols(query: string, limit?: number): Promise<SymbolHit[]> {
  return invoke<SymbolHit[]>("query_symbols", { query, limit });
}

export function indexStatus(): Promise<IndexStatus> {
  return invoke<IndexStatus>("index_status");
}

export function listenIndexProgress(
  handler: (p: IndexProgress) => void,
): Promise<() => void> {
  return getCurrentWebviewWindow().listen<IndexProgress>(
    "index:progress",
    (e) => handler(e.payload),
  );
}

export function listenIndexDone(
  handler: (s: IndexStatus) => void,
): Promise<() => void> {
  return getCurrentWebviewWindow().listen<IndexStatus>("index:done", (e) =>
    handler(e.payload),
  );
}

export function listenIndexUpdated(
  handler: (paths: string[]) => void,
): Promise<() => void> {
  return getCurrentWebviewWindow().listen<{ paths: string[] }>(
    "index:updated",
    (e) => handler(e.payload.paths),
  );
}
```

> Field-casing contract: the Rust `SymbolHit`/`IndexStatus`/`ProgressPayload` all carry `#[serde(rename_all = "camelCase")]`, so the wire keys are `startLine`/`endLine`/`fileCount`/`symbolCount` — matching the interfaces above. (Note Plan 3's `FileSymbol` returned by `index_file_symbols` is the per-file primitive; `SymbolHit` here additionally carries `path` for project-wide results.)

- [ ] **Step 2: Create the Zustand progress store `src/modules/index/store.ts`.**
```ts
import { create } from "zustand";

export type IndexPhase = "idle" | "indexing" | "ready";

interface IndexState {
  phase: IndexPhase;
  indexed: number;
  total: number;
  fileCount: number;
  symbolCount: number;
  startIndexing: () => void;
  setProgress: (indexed: number, total: number) => void;
  setReady: (fileCount: number, symbolCount: number) => void;
}

export const useIndexStore = create<IndexState>((set) => ({
  phase: "idle",
  indexed: 0,
  total: 0,
  fileCount: 0,
  symbolCount: 0,
  startIndexing: () => set({ phase: "indexing", indexed: 0, total: 0 }),
  setProgress: (indexed, total) => set({ phase: "indexing", indexed, total }),
  setReady: (fileCount, symbolCount) =>
    set({ phase: "ready", fileCount, symbolCount }),
}));
```

- [ ] **Step 3: Create the hook `src/modules/index/useProjectIndex.ts`.**
```ts
import { useEffect } from "react";
import {
  indexProject,
  listenIndexDone,
  listenIndexProgress,
  listenIndexUpdated,
  indexStatus,
} from "./project";
import { useIndexStore } from "./store";

export function useProjectIndex(root: string | null): void {
  const startIndexing = useIndexStore((s) => s.startIndexing);
  const setProgress = useIndexStore((s) => s.setProgress);
  const setReady = useIndexStore((s) => s.setReady);

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    let active = true;
    void listenIndexProgress((p) => setProgress(p.indexed, p.total)).then((u) => {
      if (active) unlisteners.push(u);
      else u();
    });
    void listenIndexDone((s) => setReady(s.fileCount, s.symbolCount)).then((u) => {
      if (active) unlisteners.push(u);
      else u();
    });
    void listenIndexUpdated(() => {
      void indexStatus().then((s) => setReady(s.fileCount, s.symbolCount));
    }).then((u) => {
      if (active) unlisteners.push(u);
      else u();
    });
    return () => {
      active = false;
      for (const u of unlisteners) u();
    };
  }, [setProgress, setReady]);

  useEffect(() => {
    if (!root) return;
    startIndexing();
    void indexProject(root).catch(() => {});
  }, [root, startIndexing]);
}
```

- [ ] **Step 4: Create the indicator `src/modules/index/IndexStatusItem.tsx`.**
```tsx
import { useIndexStore } from "./store";

export function IndexStatusItem() {
  const phase = useIndexStore((s) => s.phase);
  const indexed = useIndexStore((s) => s.indexed);
  const total = useIndexStore((s) => s.total);
  const symbolCount = useIndexStore((s) => s.symbolCount);

  if (phase === "idle") return null;

  const label =
    phase === "indexing"
      ? total > 0
        ? `Indexing… ${indexed}/${total}`
        : "Indexing…"
      : `Indexed · ${symbolCount.toLocaleString()} symbols`;

  return (
    <span className="flex shrink-0 cursor-default items-center gap-1 text-[10.5px] text-muted-foreground">
      {label}
    </span>
  );
}
```

- [ ] **Step 5: Update the barrel `src/modules/index/index.ts`.** Replace its contents with:
```ts
export { listFileSymbols } from "./symbols";
export type { FileSymbol, SymbolKind } from "./symbols";
export {
  indexProject,
  querySymbols,
  indexStatus,
  listenIndexProgress,
  listenIndexDone,
  listenIndexUpdated,
} from "./project";
export type { SymbolHit, IndexStatus, IndexProgress } from "./project";
export { useIndexStore } from "./store";
export { useProjectIndex } from "./useProjectIndex";
export { IndexStatusItem } from "./IndexStatusItem";
```

- [ ] **Step 6: Wire into the status bar.** In `src/modules/statusbar/StatusBar.tsx`, add the import near the other module imports:
```tsx
import { IndexStatusItem, useProjectIndex } from "@/modules/index";
```
Inside the `StatusBar` component body, after the `openPanel` line, add:
```tsx
  useProjectIndex(cwd);
```
Then render the indicator in the left cluster — add `<IndexStatusItem />` immediately after the `<CwdBreadcrumb ... />` element (before the `{privateActive ? (` block):
```tsx
        <IndexStatusItem />
```

- [ ] **Step 7: Typecheck**
Run: `pnpm check-types`
Expected: no type errors. If `zustand`'s `create` import path differs, match an existing store (e.g. `src/modules/spaces/lib/useSpaces.ts` or `src/modules/ai/store/agentsStore.ts`) and report the deviation.

- [ ] **Step 8: Commit**
```bash
git add src/modules/index/ src/modules/statusbar/StatusBar.tsx
git commit -m "feat(index): frontend project-index wrappers, progress store, and status indicator"
```

---

## Self-Review

**Spec coverage (against `ken-ide-implementation-plans.md` Plan 4 — "Index on project open with progress; query symbols project-wide. Done when: open a project, watch progress, query any symbol"):**
- Index on project open: `index_project` clears + walks via `run_index` in a background thread (Tasks 2–4). ✓
- With progress: `index:progress` events throttled every 50 files + `index:done`; frontend store + `IndexStatusItem` render it (Tasks 3, 5). ✓
- Query symbols project-wide: `query_symbols` over the in-memory `IndexStore` using `fuzzy_rank` (Tasks 1, 3). ✓
- Open / watch / query end-to-end: `useProjectIndex(cwd)` mounted in the status bar triggers indexing and surfaces progress; `querySymbols` is exported for Plan 6's Search Everywhere to consume. ✓
- Incremental on save (user-chosen scope): recursive `notify` watcher + `apply_change` + `index:updated`, store refreshed on change (Task 3, 5). ✓
- Design §5.3 alignment: hot data in an in-memory map; symbols+files only (no edges = Plan 7); no persistence (= Plan 5); `.ts/.mts/.cts` only. No overreach into LSP/graph territory. ✓

**Placeholder scan:** No "TBD"/"handle edge cases"/"similar to Task N". Every code step shows complete code. The verification asides (crate API signatures, `zustand` import path) are concrete cross-checks against named existing files with a stated default, not placeholders. ✓

**Type consistency:** `Symbol`/`SymbolKind` reused unchanged from Plan 3. `SymbolHit { name, kind, path, start_line, end_line }`, `IndexStatus { root, file_count, symbol_count }`, `ProgressPayload { indexed, total }`, `UpdatedPayload { paths }` defined once and used identically across `query`, the command, the integration test, and the TS interfaces (camelCase on the wire). The command names `index_project` / `query_symbols` / `index_status` match between the Rust commands, the `generate_handler!` registration, and the `invoke(...)` calls. Event names `index:progress` / `index:done` / `index:updated` match between Rust `emit` and TS `listen`. `fuzzy_rank` reused from Plan 1 with its exact signature. `run_index(root, store, on_progress)` signature is identical in Tasks 2, 3 (command), and 4 (integration test). ✓

**Known risks flagged in-plan:** (1) `registry.authorize` signature (Task 3 Step 5 cross-checks `workspace.rs`); (2) `ignore::WalkBuilder` `filter_entry`/`flatten` API (Task 2 cross-checks `fs/search.rs`); (3) accessing managed `IndexStore` from spawned threads via `app.state::<IndexStore>()` (the standard Tauri pattern — `AppHandle` is `Send + 'static`); (4) the recursive watcher is stored in `IndexWatchState` so it is not dropped (dropping a `notify` watcher stops it) and is replaced on each `index_project` call. Query cost over a large flat symbol list is acceptable for Plan 4 (Plan 1 bench: ~1.46ms/10k keys) and is optimized later by persistence (Plan 5) and Search Everywhere (Plan 6).
