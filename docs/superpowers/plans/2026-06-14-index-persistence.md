# Index Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist the in-memory `IndexStore` to a memory-mapped, checksummed, versioned on-disk file so reopening a project loads its symbols near-instantly (cold-start gate) instead of re-walking and re-parsing the whole tree, reconciling staleness via per-file mtimes.

**Architecture:** A new pure `index/persist.rs` owns the on-disk format (rkyv body + `[magic|version|crc32]` header, atomic temp-file rename, `memmap2` read). `IndexStore` gains a per-file mtime (`FileEntry { mtime_ms, symbols }`) plus `snapshot()`/`load_snapshot()`. `index/project.rs` gains `file_mtime_ms`, an mtime-diff `reconcile()`, and `load_or_index()` (load+reconcile on hit, full `run_index` on miss, then save); the `index_project` command resolves a per-root cache path under the OS cache dir and the watcher drain triggers a debounced save.

**Tech Stack:** Rust, `rkyv` 0.8 (zero-copy archive), `memmap2` 0.9, `crc32fast` 1, existing `tree-sitter` symbols (Plan 3), `ignore::WalkBuilder`/`notify` (Plan 4), Tauri path API (`app.path().app_cache_dir()`).

**Scope discipline (locked decisions from `docs/superpowers/specs/2026-06-14-index-persistence-design.md`):**
- **Format = rkyv + memmap2.** Body validated via crc32 + rkyv `bytecheck`.
- **Staleness = cheap mtime reconcile.** Stat current files; reparse only newer/new; drop deleted.
- **Location = global OS cache dir, keyed by canonical-root hash.** Never written into the repo.
- **TypeScript `.ts`/`.mts`/`.cts` only; symbols + files only** (no graph edges — Plan 7).
- **Sync core + std threads** (no tokio); no `tauri-specta`; no code comments (project rule).
- **Deferred:** zero-copy *querying* against the live mmap (we deserialize into the owned store on load); cache-dir eviction/size cap.

---

## File Structure

| File | Responsibility |
|---|---|
| `src-tauri/Cargo.toml` | Add `rkyv = "0.8"`, `memmap2 = "0.9"`, `crc32fast = "1"`; register the `persist_bench` benchmark |
| `src-tauri/src/modules/index/symbols.rs` | Add rkyv `Archive`/`Serialize`/`Deserialize` derives to `Symbol` + `SymbolKind` (keep existing serde derives) |
| `src-tauri/src/modules/index/persist.rs` | **New.** `PersistedIndex`/`PersistedFile` types, `save`/`load`, header+crc+atomic write+mmap read; unit tests |
| `src-tauri/src/modules/index/mod.rs` | Register `pub mod persist;` |
| `src-tauri/src/modules/index/store.rs` | `FileEntry { mtime_ms, symbols }`; `replace_file(path, mtime_ms, symbols)`; `snapshot()`/`load_snapshot()`; update 5 tests |
| `src-tauri/src/modules/index/project.rs` | `file_mtime_ms`, mtime capture in `run_index`/`apply_change`, `reconcile`, `load_or_index`, cache-path resolution, `index_project` wiring, debounced save in `watch_drain` |
| `src-tauri/tests/index_persist.rs` | **New.** Integration: full-index → save → reconcile (edit/add/delete + sentinel no-reparse) |
| `src-tauri/benches/persist_bench.rs` | **New.** Criterion save+load round-trip on a synthetic multi-thousand-file index |

**On-disk file layout (`persist.rs`):**
```
offset 0  : magic   = b"KENIDX01"   (8 bytes; doubles as format tag)
offset 8  : version = u32 LE        (CURRENT_VERSION; bump invalidates all caches)
offset 12 : crc32   = u32 LE        (crc32fast over the rkyv body only)
offset 16 : rkyv body ...           (16-byte header keeps the body 16-aligned for zero-copy access)
```
The 16-byte header is deliberate: a `memmap2` mapping is page-aligned (≥4096), so a body starting at offset 16 is 16-byte aligned — satisfying rkyv's archive-root alignment requirement and allowing `rkyv::access` directly on the mapped bytes.

---

### Task 1: Dependencies, rkyv derives, and the `persist.rs` format (TDD)

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/modules/index/symbols.rs:4` and `:15` (derive lines)
- Modify: `src-tauri/src/modules/index/mod.rs`
- Create: `src-tauri/src/modules/index/persist.rs`

- [ ] **Step 1: Add dependencies.** In `src-tauri/Cargo.toml`, in the `[dependencies]` section (e.g. right after the `tree-sitter-typescript = "0.23"` line), add:
```toml
rkyv = "0.8"
memmap2 = "0.9"
crc32fast = "1"
```

- [ ] **Step 2: Add rkyv derives to the symbol types.** In `src-tauri/src/modules/index/symbols.rs`, change the derive line above `pub enum SymbolKind` (currently `#[derive(Serialize, Clone, Debug, PartialEq, Eq)]` at line 4) to:
```rust
#[derive(Serialize, Clone, Debug, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
```
and make the identical change to the derive line above `pub struct Symbol` (line 15). The fully-pathed `rkyv::Serialize`/`rkyv::Deserialize` do not collide with the imported serde `Serialize`. Leave `#[serde(rename_all = "camelCase")]` and all fields unchanged.

- [ ] **Step 3: Register the module.** In `src-tauri/src/modules/index/mod.rs`, add `pub mod persist;` so the file reads (alphabetical):
```rust
pub mod persist;
pub mod project;
pub mod store;
pub mod symbols;
```

- [ ] **Step 4: Write `persist.rs` with the types, format, and failing-first tests.** Create `src-tauri/src/modules/index/persist.rs`:
```rust
use std::path::Path;

use crate::modules::index::symbols::Symbol;

const MAGIC: &[u8; 8] = b"KENIDX01";
const CURRENT_VERSION: u32 = 1;
const HEADER_LEN: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct PersistedFile {
    pub path: String,
    pub mtime_ms: u64,
    pub symbols: Vec<Symbol>,
}

#[derive(Clone, Debug, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct PersistedIndex {
    pub version: u32,
    pub root: String,
    pub files: Vec<PersistedFile>,
}

fn tmp_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().map(|s| s.to_os_string()).unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

pub fn save(path: &Path, index: &PersistedIndex) -> std::io::Result<()> {
    let body = rkyv::to_bytes::<rkyv::rancor::Error>(index)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let crc = crc32fast::hash(&body);
    let mut buf = Vec::with_capacity(HEADER_LEN + body.len());
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&CURRENT_VERSION.to_le_bytes());
    buf.extend_from_slice(&crc.to_le_bytes());
    buf.extend_from_slice(&body);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(path);
    std::fs::write(&tmp, &buf)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load(path: &Path) -> Option<PersistedIndex> {
    let file = std::fs::File::open(path).ok()?;
    let mmap = unsafe { memmap2::Mmap::map(&file).ok()? };
    let bytes: &[u8] = &mmap;
    if bytes.len() < HEADER_LEN || &bytes[0..8] != MAGIC {
        return None;
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
    if version != CURRENT_VERSION {
        return None;
    }
    let crc = u32::from_le_bytes(bytes[12..16].try_into().ok()?);
    let body = &bytes[HEADER_LEN..];
    if crc32fast::hash(body) != crc {
        return None;
    }
    let archived = rkyv::access::<ArchivedPersistedIndex, rkyv::rancor::Error>(body).ok()?;
    rkyv::deserialize::<PersistedIndex, rkyv::rancor::Error>(archived).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::index::symbols::SymbolKind;

    fn sample() -> PersistedIndex {
        PersistedIndex {
            version: CURRENT_VERSION,
            root: "/proj".to_string(),
            files: vec![PersistedFile {
                path: "/proj/a.ts".to_string(),
                mtime_ms: 1234,
                symbols: vec![Symbol {
                    name: "greet".to_string(),
                    kind: SymbolKind::Function,
                    start_line: 1,
                    end_line: 3,
                }],
            }],
        }
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.kenidx");
        let index = sample();
        save(&path, &index).unwrap();
        assert_eq!(load(&path), Some(index));
    }

    #[test]
    fn atomic_write_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.kenidx");
        save(&path, &sample()).unwrap();
        assert!(path.exists());
        assert!(!tmp_path(&path).exists());
    }

    #[test]
    fn missing_file_loads_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(&dir.path().join("nope.kenidx")), None);
    }

    #[test]
    fn version_mismatch_loads_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.kenidx");
        save(&path, &sample()).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[8] = bytes[8].wrapping_add(1);
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(load(&path), None);
    }

    #[test]
    fn corrupted_body_loads_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.kenidx");
        save(&path, &sample()).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] = bytes[last].wrapping_add(1);
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(load(&path), None);
    }
}
```

- [ ] **Step 5: Run the tests.**
Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib index::persist`
Expected: 5 tests pass.

**rkyv 0.8 API cross-check (do this if it does not compile/pass):** the exact spellings of four things may differ by patch version — adapt minimally and report any deviation:
1. The derive macros: `rkyv::Archive` / `rkyv::Serialize` / `rkyv::Deserialize` (ensure rkyv's default features, which include `bytecheck`, are on — they are by default).
2. `rkyv::to_bytes::<rkyv::rancor::Error>(index)` returning an `AlignedVec` (deref-coerces to `&[u8]` for `crc32fast::hash` and `extend_from_slice`).
3. `rkyv::access::<ArchivedPersistedIndex, rkyv::rancor::Error>(body)` — the derive generates the archived type named `ArchivedPersistedIndex`. If that name is not in scope, use `rkyv::access::<rkyv::Archived<PersistedIndex>, rkyv::rancor::Error>(body)`.
4. `rkyv::deserialize::<PersistedIndex, rkyv::rancor::Error>(archived)` — if it wants a reference, pass `&archived` (it is already `&ArchivedPersistedIndex` from `access`, so this should match).
If `access` panics/errors on alignment (it should not, given the 16-byte header on a page-aligned mmap), the robust fallback is to copy the body into an aligned buffer first: `let mut aligned = rkyv::util::AlignedVec::<16>::new(); aligned.extend_from_slice(body);` then `rkyv::access::<_, _>(&aligned)`. Report if you needed this.

- [ ] **Step 6: Commit**
```bash
git add src-tauri/Cargo.toml src-tauri/src/modules/index/symbols.rs src-tauri/src/modules/index/mod.rs src-tauri/src/modules/index/persist.rs
git commit -m "feat(index): rkyv on-disk index format with checksum and version header

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Per-file mtime in the store + keep the crate compiling

This task changes `replace_file`'s signature (adds `mtime_ms`) and therefore **must** also update the two production callers in `project.rs` in the same commit, or the crate will not build. Pure store logic + a snapshot/load round-trip.

**Files:**
- Modify: `src-tauri/src/modules/index/store.rs`
- Modify: `src-tauri/src/modules/index/project.rs:60-69` (`run_index`) and `:74-83` (`apply_change`)

- [ ] **Step 1: Rewrite `store.rs`.** Replace the contents of `src-tauri/src/modules/index/store.rs` with:
```rust
use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Serialize;

use crate::modules::fs::search::fuzzy_rank;
use crate::modules::index::persist::{PersistedFile, PersistedIndex};
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileEntry {
    mtime_ms: u64,
    symbols: Vec<Symbol>,
}

#[derive(Default)]
struct IndexData {
    root: Option<String>,
    files: BTreeMap<String, FileEntry>,
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

    pub fn replace_file(&self, path: String, mtime_ms: u64, symbols: Vec<Symbol>) {
        self.inner
            .lock()
            .expect("index store poisoned")
            .files
            .insert(path, FileEntry { mtime_ms, symbols });
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
            symbol_count: data.files.values().map(|e| e.symbols.len()).sum(),
        }
    }

    pub fn query(&self, query: &str, limit: usize) -> Vec<SymbolHit> {
        let data = self.inner.lock().expect("index store poisoned");
        let mut entries: Vec<(&str, &Symbol)> = Vec::new();
        for (path, entry) in data.files.iter() {
            for s in &entry.symbols {
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

    pub fn snapshot(&self) -> PersistedIndex {
        let data = self.inner.lock().expect("index store poisoned");
        let files = data
            .files
            .iter()
            .map(|(path, entry)| PersistedFile {
                path: path.clone(),
                mtime_ms: entry.mtime_ms,
                symbols: entry.symbols.clone(),
            })
            .collect();
        PersistedIndex {
            version: 1,
            root: data.root.clone().unwrap_or_default(),
            files,
        }
    }

    pub fn load_snapshot(&self, index: PersistedIndex) {
        let mut data = self.inner.lock().expect("index store poisoned");
        data.root = if index.root.is_empty() {
            None
        } else {
            Some(index.root)
        };
        data.files = index
            .files
            .into_iter()
            .map(|f| {
                (
                    f.path,
                    FileEntry {
                        mtime_ms: f.mtime_ms,
                        symbols: f.symbols,
                    },
                )
            })
            .collect();
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
        store.replace_file("a.ts".to_string(), 0, vec![sym("greet", 1), sym("run", 5)]);
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
        store.replace_file("a.ts".to_string(), 0, vec![sym("old", 1)]);
        store.replace_file("a.ts".to_string(), 0, vec![sym("new", 2)]);
        assert_eq!(store.status().symbol_count, 1);
        assert!(store.query("old", 10).is_empty());
        assert_eq!(store.query("new", 10).len(), 1);
    }

    #[test]
    fn remove_file_drops_its_symbols() {
        let store = IndexStore::default();
        store.replace_file("a.ts".to_string(), 0, vec![sym("greet", 1)]);
        store.remove_file("a.ts");
        assert_eq!(store.status().file_count, 0);
        assert!(store.query("greet", 10).is_empty());
    }

    #[test]
    fn query_respects_limit() {
        let store = IndexStore::default();
        store.replace_file(
            "a.ts".to_string(),
            0,
            vec![sym("handle", 1), sym("handler", 2), sym("handlers", 3)],
        );
        assert_eq!(store.query("handle", 2).len(), 2);
    }

    #[test]
    fn clear_resets_root_and_files() {
        let store = IndexStore::default();
        store.set_root(Some("/proj".to_string()));
        store.replace_file("a.ts".to_string(), 0, vec![sym("greet", 1)]);
        store.clear();
        let status = store.status();
        assert_eq!(status.root, None);
        assert_eq!(status.file_count, 0);
    }

    #[test]
    fn snapshot_then_load_snapshot_round_trips() {
        let store = IndexStore::default();
        store.set_root(Some("/proj".to_string()));
        store.replace_file("/proj/a.ts".to_string(), 99, vec![sym("greet", 1)]);
        let snap = store.snapshot();

        let restored = IndexStore::default();
        restored.load_snapshot(snap);
        assert_eq!(restored.status().root, Some("/proj".to_string()));
        assert_eq!(restored.status().file_count, 1);
        assert_eq!(restored.query("greet", 10).len(), 1);
    }
}
```

- [ ] **Step 2: Update the two `replace_file` callers in `project.rs` so the crate compiles.** In `src-tauri/src/modules/index/project.rs`, change `run_index` (lines 60-69) to capture and pass mtime:
```rust
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
```
and change `apply_change` (lines 74-83) to:
```rust
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
```

- [ ] **Step 3: Add the `file_mtime_ms` helper to `project.rs`.** Immediately after the `is_indexable` function (before `collect_indexable_files`), add:
```rust
pub fn file_mtime_ms(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let dur = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(dur.as_millis() as u64)
}
```
`std::time::UNIX_EPOCH` is reachable via the existing `use std::time::{Duration, Instant};` import path's crate root, but to be safe add `use std::time::UNIX_EPOCH;` is **not** needed — `std::time::UNIX_EPOCH` is referenced fully-qualified above. Confirm `project.rs` already imports `std::path::Path` (it does, line 2).

- [ ] **Step 4: Run the store + project unit tests.**
Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib index::store && cargo test --manifest-path src-tauri/Cargo.toml --lib index::project`
Expected: store = 7 tests pass (6 original + `snapshot_then_load_snapshot_round_trips`); project = 4 tests pass (the existing Plan 4 tests still pass unchanged — mtime is internal, queries are by name).

- [ ] **Step 5: Build the whole crate to confirm nothing else broke.**
Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: compiles.

- [ ] **Step 6: Commit**
```bash
git add src-tauri/src/modules/index/store.rs src-tauri/src/modules/index/project.rs
git commit -m "feat(index): track per-file mtime and add store snapshot/load_snapshot

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: `reconcile` + `load_or_index` (TDD)

**Files:**
- Modify: `src-tauri/src/modules/index/project.rs`

- [ ] **Step 1: Write the failing test.** Add this test inside the existing `mod tests` block in `src-tauri/src/modules/index/project.rs` (the block that already has the `write` helper). It seeds the cache with a sentinel symbol that real parsing would never produce, for an unchanged file (`keep.ts`, cached mtime == its real mtime), and asserts the sentinel survives (proving no reparse). `change.ts` is given a stale cached mtime of `0` and rewritten on disk, so it is reparsed; `gone.ts` is only in the cache (no file on disk) so it is dropped; `new.ts` exists on disk but not in the cache so it is added:
```rust
    #[test]
    fn reconcile_keeps_unchanged_reparses_changed_adds_new_drops_deleted() {
        use crate::modules::index::persist::{PersistedFile, PersistedIndex};
        use crate::modules::index::symbols::SymbolKind;

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
```

- [ ] **Step 2: Run it to confirm it fails.**
Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib index::project::tests::reconcile_keeps_unchanged_reparses_changed_adds_new_drops_deleted`
Expected: FAIL — `reconcile` does not exist (compile error).

- [ ] **Step 3: Implement `reconcile` and `load_or_index`.** First extend the imports at the top of `src-tauri/src/modules/index/project.rs`: add these lines to the existing `use` block:
```rust
use std::collections::HashMap;

use crate::modules::index::persist::{self, PersistedIndex};
```
(`std::collections::HashSet` is already imported; add `HashMap` alongside it — combine into `use std::collections::{HashMap, HashSet};` if you prefer.)

Then add the following after `run_index` (and after `file_mtime_ms`), before the `DEBOUNCE`/`apply_change` section:
```rust
pub fn reconcile(
    root: &Path,
    loaded: PersistedIndex,
    store: &IndexStore,
    mut on_progress: impl FnMut(usize, usize),
) {
    let cached: HashMap<String, crate::modules::index::persist::PersistedFile> = loaded
        .files
        .into_iter()
        .map(|f| (f.path.clone(), f))
        .collect();
    let files = collect_indexable_files(root);
    let total = files.len();
    for (i, path) in files.iter().enumerate() {
        let key = to_canon(path);
        let current = file_mtime_ms(path).unwrap_or(u64::MAX);
        match cached.get(&key) {
            Some(entry) if entry.mtime_ms >= current => {
                store.replace_file(key, entry.mtime_ms, entry.symbols.clone());
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
```

- [ ] **Step 4: Run the test, confirm it passes.**
Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib index::project::tests::reconcile_keeps_unchanged_reparses_changed_adds_new_drops_deleted`
Expected: PASS.

- [ ] **Step 5: Run the whole project module + build.**
Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib index::project && cargo build --manifest-path src-tauri/Cargo.toml`
Expected: project = 5 tests pass; crate compiles (`load_or_index`/`reconcile` not yet called from the command — that is Task 4 — but they must compile).

- [ ] **Step 6: Commit**
```bash
git add src-tauri/src/modules/index/project.rs
git commit -m "feat(index): mtime reconcile and load_or_index over the persisted cache

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Wire persistence into the command + debounced save in the watcher

**Files:**
- Modify: `src-tauri/src/modules/index/project.rs` (`index_project`, `watch_drain`; add `cache_path_for`)

- [ ] **Step 1: Add the cache-path resolver.** In `src-tauri/src/modules/index/project.rs`, add this function near the top-level functions (after `load_or_index`). It uses the Tauri path API (the file already imports `tauri::{AppHandle, Emitter, Manager, State}`; `Manager` brings `app.path()` into scope):
```rust
fn cache_path_for(app: &AppHandle, canonical_root: &str) -> Option<std::path::PathBuf> {
    let base = app.path().app_cache_dir().ok()?;
    let hash = crc32fast::hash(canonical_root.as_bytes());
    let name = Path::new(canonical_root)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("root");
    Some(base.join("index").join(format!("{name}-{hash:08x}.kenidx")))
}
```

- [ ] **Step 2: Rewrite the `index_project` build thread to load-or-index.** Replace the body of `index_project` (currently lines ~172-206, the part from `store.clear();` to the final `Ok(())`) with:
```rust
    store.clear();
    let canon_root = to_canon(&root_path);
    store.set_root(Some(canon_root.clone()));
    start_watch(&watch, &app, &root_path)?;

    let cache_path = cache_path_for(&app, &canon_root);
    let app_for_thread = app.clone();
    std::thread::Builder::new()
        .name("ken-index-build".into())
        .spawn(move || {
            let store = app_for_thread.state::<IndexStore>();
            let mut last_emit = 0usize;
            let on_progress = |indexed: usize, total: usize| {
                if indexed == total || indexed - last_emit >= 50 {
                    last_emit = indexed;
                    let _ = app_for_thread
                        .emit("index:progress", ProgressPayload { indexed, total });
                }
            };
            match &cache_path {
                Some(p) => load_or_index(&root_path, p, &store, on_progress),
                None => run_index(&root_path, &store, on_progress),
            }
            let status = store.status();
            let _ = app_for_thread.emit("index:done", status);
        })
        .map_err(|e| e.to_string())?;
    Ok(())
```
Leave the command signature and the `registry.authorize` / `is_dir` guard above it unchanged.

- [ ] **Step 3: Add the debounced save to `watch_drain`.** In `src-tauri/src/modules/index/project.rs`, in `watch_drain`, replace the final emit block:
```rust
        if !changed.is_empty() {
            let _ = app.emit("index:updated", UpdatedPayload { paths: changed });
        }
```
with:
```rust
        if !changed.is_empty() {
            let _ = app.emit("index:updated", UpdatedPayload { paths: changed });
            if let Some(root) = store.status().root {
                if let Some(p) = cache_path_for(&app, &root) {
                    let _ = persist::save(&p, &store.snapshot());
                }
            }
        }
```
The drain is already debounced (150 ms / 1000 ms window), so this saves at most once per change batch.

- [ ] **Step 4: Build + run the full index module.**
Run: `cargo build --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml --lib index`
Expected: compiles; all `index::store` / `index::project` / `index::persist` unit tests pass.

- [ ] **Step 5: Commit**
```bash
git add src-tauri/src/modules/index/project.rs
git commit -m "feat(index): load-or-index on open and debounced cache save on change

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: Integration test + cold-start tripwire bench

**Files:**
- Create: `src-tauri/tests/index_persist.rs`
- Create: `src-tauri/benches/persist_bench.rs`
- Modify: `src-tauri/Cargo.toml` (register the bench)

- [ ] **Step 1: Write the integration test.** Create `src-tauri/tests/index_persist.rs`:
```rust
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
```
Note: this assumes `terax_lib::modules::fs::to_canon` is public (it is used across `fs`/`index`; confirm by reading the import in `src-tauri/tests/index_project.rs` and `src-tauri/src/modules/index/project.rs` which both use `crate::modules::fs::to_canon`). If `to_canon` is not re-exported at `terax_lib::modules::fs::to_canon`, replace the test's `to_canon` helper with `std::fs::canonicalize(p).unwrap().to_string_lossy().to_string()` and report the deviation.

- [ ] **Step 2: Write the benchmark.** Create `src-tauri/benches/persist_bench.rs`:
```rust
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
```

- [ ] **Step 3: Register the bench.** In `src-tauri/Cargo.toml`, after the existing `[[bench]]` block for `search_bench`, add:
```toml
[[bench]]
name = "persist_bench"
harness = false
```

- [ ] **Step 4: Run the full test suite.**
Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: all tests pass, including the three new integration tests in `index_persist.rs` and no regressions in the Plan 4 suite.

- [ ] **Step 5: Verify the bench compiles and runs.**
Run: `cargo bench --manifest-path src-tauri/Cargo.toml --bench persist_bench -- --warm-up-time 1 --measurement-time 2`
Expected: it builds and reports `persist_load_5k` / `persist_save_5k` timings (numbers are informational; the hard gate is the `cold_start_load_is_fast` test).

- [ ] **Step 6: Commit**
```bash
git add src-tauri/tests/index_persist.rs src-tauri/benches/persist_bench.rs src-tauri/Cargo.toml
git commit -m "test(index): persistence integration tests and cold-start bench

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage (against `docs/superpowers/specs/2026-06-14-index-persistence-design.md`):**
- rkyv + memmap2 format with `[magic|version|crc32]` header + atomic temp-rename: Task 1 (`persist.rs` `save`/`load`). ✓
- `Symbol`/`SymbolKind` rkyv derives: Task 1 Step 2. ✓
- Per-file mtime in store (`FileEntry`), `replace_file(path, mtime_ms, symbols)`, `snapshot`/`load_snapshot`: Task 2. ✓
- `file_mtime_ms`, mtime capture in `run_index`/`apply_change`: Task 2 Steps 2-3. ✓
- mtime `reconcile` (keep unchanged / reparse newer+new / drop deleted) + `load_or_index`: Task 3. ✓
- Global cache dir keyed by canonical-root hash (`app_cache_dir`/crc32): Task 4 Step 1. ✓
- `index_project` load-or-index + final save: Task 4 Step 2. ✓
- Debounced save in watcher drain: Task 4 Step 3. ✓
- Error handling = any load failure → `None` → full rebuild; save failure ignored; `app_cache_dir` unavailable → in-memory only: encoded in `load` returning `Option`, `let _ = persist::save(...)`, and the `Some(p)/None` match in Task 4 Step 2. ✓
- Tests: persist unit (round-trip/version/corruption/missing/atomic) Task 1; store snapshot round-trip Task 2; reconcile sentinel Task 3; integration warm-reopen + cold-start tripwire + full-index-reload Task 5; criterion bench Task 5. ✓

**Placeholder scan:** No "TBD"/"handle edge cases"/"similar to Task N". Every code step shows complete code. The rkyv-API and `to_canon`-visibility asides are concrete cross-checks against named files with explicit fallbacks and a stated default, not placeholders. ✓

**Type consistency:**
- `replace_file(path: String, mtime_ms: u64, symbols: Vec<Symbol>)` — defined Task 2, called identically in `run_index`/`apply_change` (Task 2), `reconcile` (Task 3). ✓
- `PersistedIndex { version: u32, root: String, files: Vec<PersistedFile> }` and `PersistedFile { path: String, mtime_ms: u64, symbols: Vec<Symbol> }` — defined Task 1, consumed unchanged by `snapshot`/`load_snapshot` (Task 2), `reconcile`/`load_or_index` (Task 3), and the tests/bench (Task 5). ✓
- `save(&Path, &PersistedIndex) -> io::Result<()>` / `load(&Path) -> Option<PersistedIndex>` — defined Task 1, used identically in Tasks 3-5. ✓
- `reconcile(root, loaded, store, on_progress)` / `load_or_index(root, cache_path, store, on_progress)` signatures identical across Task 3 definition, Task 4 call, and Task 5 integration test. ✓
- `cache_path_for(&AppHandle, &str) -> Option<PathBuf>` — defined Task 4 Step 1, used in `index_project` (Step 2) and `watch_drain` (Step 3). ✓
- Event names (`index:progress`/`index:done`/`index:updated`) and `ProgressPayload`/`UpdatedPayload` unchanged from Plan 4; frontend needs no change. ✓

**Known risks flagged in-plan:** (1) rkyv 0.8 exact API spellings — Task 1 Step 5 gives concrete fallbacks (archived-type name, `AlignedVec` access path) and a report instruction; (2) mmap alignment — addressed structurally by the 16-byte header on a page-aligned mapping, with an `AlignedVec` fallback documented; (3) `to_canon` public visibility from the integration test — Task 5 Step 1 gives a `std::fs::canonicalize` fallback; (4) the existing Plan 4 `apply_change` test (`apply_change_upserts_and_removes`) still passes because mtime is internal and that test queries by symbol name (verified by Task 2 Step 4). Re-serializing the whole index per watcher batch is acceptable at this scope and is the documented deferred optimization.
