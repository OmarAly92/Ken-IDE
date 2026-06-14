# Index Persistence Design (Plan 5)

**Status:** Approved (2026-06-14)
**Milestone:** 1 — The Index. Follows Plan 4 (project-wide index + background indexing).

## Goal

Persist the in-memory `IndexStore` to disk so that reopening a project loads its symbols near-instantly instead of re-walking and re-parsing the whole tree, while staying correct when files changed on disk while the app was closed.

Roadmap "done when": *Reopen project loads index from disk fast (cold-start gate).*

## Locked decisions

- **Format: rkyv + memmap2.** Zero-copy archive on disk; memory-map the file and validate/access via rkyv's `bytecheck`. (Roadmap default.)
- **Staleness: cheap mtime reconcile.** On load, stat every currently-indexable file and re-parse only those whose mtime is newer than the saved snapshot; drop deleted files; add new ones. Per-file mtime is stored in the index.
- **Location: global OS cache dir, keyed by canonical root hash.** Does not pollute the repo.
- **No `tauri-specta`** (hand-typed IPC, consistent with Plans 3–4).
- **TypeScript `.ts`/`.mts`/`.cts` only**, symbols + files only (no graph edges — Plan 7).
- **Sync core + std threads** (no tokio), matching Plan 4.

## Architecture

Three units, each independently testable.

### 1. `src-tauri/src/modules/index/persist.rs` (new) — on-disk format + atomic IO

Pure: operates on a cache-file `Path` and a snapshot value; no Tauri, no project knowledge.

**File layout:**
```
[ magic: 7 bytes "KENIDX\0" ]
[ version: u32 little-endian ]
[ crc32: u32 little-endian  ]   # crc32fast over the rkyv body only
[ rkyv body ...             ]
```

**Persisted types** (also derive rkyv `Archive`/`Serialize`/`Deserialize`):
```
PersistedIndex { version: u32, root: String, files: Vec<PersistedFile> }
PersistedFile  { path: String, mtime_ms: u64, symbols: Vec<Symbol> }
```
`Symbol` / `SymbolKind` (from Plan 3) gain rkyv derives alongside their existing serde derives.

**API:**
- `save(path: &Path, index: &PersistedIndex) -> std::io::Result<()>` — serialize body with rkyv, compute crc32, write `magic+version+crc+body` to a sibling temp file, then atomically `rename` over the target. Creates parent dirs as needed.
- `load(path: &Path) -> Option<PersistedIndex>` — `memmap2`-map the file; validate magic, then `version == CURRENT_VERSION`, then crc32 of the body; then rkyv `access` (bytecheck) + `deserialize` into an owned `PersistedIndex`. **Any failure returns `None`.** Never panics.

`CURRENT_VERSION: u32` is a module constant; bumping it invalidates all existing caches.

### 2. `src-tauri/src/modules/index/store.rs` (modify) — add per-file mtime

- `IndexData.files` becomes `BTreeMap<String, FileEntry>` where `FileEntry { mtime_ms: u64, symbols: Vec<Symbol> }`.
- `replace_file(path: String, mtime_ms: u64, symbols: Vec<Symbol>)` — **signature change** (carries mtime). Updates the 2 production callers in `project.rs` and the 5 test call sites in `store.rs`.
- `remove_file`, `clear`, `set_root` unchanged. `status()` sums `entry.symbols.len()`; `query()` iterates `entry.symbols` — behavior unchanged, `SymbolHit` unchanged (mtime is internal).
- New `snapshot() -> PersistedIndex` — builds a `PersistedIndex` from the current root + files (sorted, deterministic via the BTreeMap).
- New `load_snapshot(index: PersistedIndex)` — replaces store contents from a loaded snapshot (sets root + files).

### 3. `src-tauri/src/modules/index/project.rs` (modify) — reconcile + wiring

- `file_mtime_ms(path: &Path) -> Option<u64>` — `metadata().modified()` → ms since UNIX epoch.
- `run_index` and `apply_change` capture the file's mtime when they read it and pass it to `replace_file`. A failed mtime stat falls back to `0` (forces reparse next reconcile — safe).
- `reconcile(root, loaded: PersistedIndex, store, on_progress)`:
  1. Build a lookup of `loaded.files` by path.
  2. Walk current indexable files (`collect_indexable_files`). For each: stat mtime; if present in `loaded` with `mtime_ms >= current` → insert cached symbols + mtime into the store (no reparse); else read + `extract_symbols` + insert with fresh mtime.
  3. Files in `loaded` not present in the current walk are simply not inserted (dropped).
  4. Report progress per file (same cadence as `run_index`).
- `load_or_index(root, cache_path, store, on_progress)`: `persist::load(cache_path)` → if `Some` and `loaded.root == canonical root` → `reconcile`; else `run_index`. After populating, `persist::save(cache_path, &store.snapshot())`.
- `index_project` (command): resolve the cache path (see below) and run `load_or_index` in its existing background thread, then emit `index:done` as today. If cache-path resolution fails, fall back to in-memory-only indexing (no persistence) — never blocks indexing.
- Watcher drain: after applying an incremental batch (and emitting `index:updated`), trigger a **debounced save** of `store.snapshot()` to the cache path. The drain is already debounced (150 ms / 1000 ms window), so one save per batch is acceptable; re-serializing the whole index per batch is fine at this scale and is noted as a future optimization.

### Cache path resolution (command layer)

`app.path().app_cache_dir()? / "index" / "<basename>-<crc32hex>.kenidx"` where `crc32hex` is the crc32 of the canonical root path and `basename` is the root's last path component (human-readable). The stored `root` is validated against the requested canonical root on load, so a hash collision is detected and treated as a miss. This resolution lives in the command/thread layer; `load_or_index`, `reconcile`, `save`, `load` all take an explicit path and are testable without Tauri.

## Data flow (reopen)

```
index_project(root)
  → authorize, clear store, set_root, start recursive watcher
  → background thread:
       cache_path = app_cache_dir/index/<...>.kenidx
       load_or_index(root, cache_path, store, on_progress):
            persist::load(cache_path)
               Some(valid, root matches) → reconcile (stat + selective reparse)
               None / invalid           → run_index (full)
            persist::save(cache_path, store.snapshot())
       emit index:done(status)
  → watcher: apply_change per batch → emit index:updated → debounced save
```

`index:progress` events fire during both reconcile and full index, identical to Plan 4. The frontend (`useProjectIndex`, `IndexStatusItem`) needs no changes — a warm reload simply shows a brief "Indexing…" then "Indexed · N symbols", much faster.

## Error handling

| Condition | Behavior |
|---|---|
| Cache file missing | `load` → `None` → full `run_index` |
| Bad magic / version mismatch / crc fail / rkyv-invalid | `load` → `None` → full rebuild (silent; corruption never crashes) |
| Stored `root` ≠ requested canonical root | Treated as miss → full rebuild |
| `stat` fails for a file during reconcile | mtime treated as `0`/newer → reparse that file |
| `save` fails (disk full, permissions) | Logged; in-memory index unaffected |
| `app_cache_dir()` unavailable | Index runs in-memory only (no persistence); no error to user |

## Dependencies to add

- `rkyv = "0.8"` — zero-copy serialization (derives on `Symbol`/`SymbolKind`/`PersistedIndex`/`PersistedFile`).
- `memmap2 = "0.9"` — memory-map the cache file for `load`.
- `crc32fast = "1"` — body checksum and cache-filename hash (one dep serves both).

## Testing

**`persist.rs` unit tests:**
- Round-trip: build a `PersistedIndex`, `save` to a tempdir path, `load`, assert structural equality.
- Version mismatch: write a file with a wrong version header → `load` returns `None`.
- Corruption: flip a byte in the body → crc mismatch → `load` returns `None`.
- Missing file → `load` returns `None`.
- Atomic write: after `save`, the target exists and no temp sibling remains.

**Reconcile integration test (`tests/index_persist.rs`):**
- Build a temp project, full-index it, `save`.
- Seed the cache for one unchanged file with a **sentinel symbol** that real parsing would never produce.
- Edit a second file (bump mtime), add a third, delete a fourth.
- `reconcile` against the saved snapshot.
- Assert: edited/added files reflect fresh parse; deleted file gone; **unchanged file still carries the sentinel symbol** (proving no reparse).

**Cold-start tripwire:**
- A `criterion` bench (mirroring Plan 1's `search_bench.rs`) measuring `save` + `load` round-trip on a synthetic multi-thousand-file index.
- A latency tripwire test asserting `load` of that index stays under a fixed budget.

## Deferred (not in Plan 5)

- Zero-copy *querying* directly against the live mmap (Plan 5 deserializes into the existing owned store on load).
- LRU eviction / size cap of the global cache directory.
- Cross-machine cache portability (paths are absolute + canonical; cache is host-local by design).
