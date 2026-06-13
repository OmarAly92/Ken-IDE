# tree-sitter Parse + Symbols (TypeScript) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Parse a TypeScript file in Rust with tree-sitter and return its symbol list (functions, classes, methods, interfaces, type aliases, enums) to the frontend through a Tauri command.

**Architecture:** A new `modules/index` backend module owns a pure `extract_symbols(source) -> Vec<Symbol>` function (tree-sitter parse + recursive named-node walk, no queries — avoids the tree-sitter query/streaming-iterator API churn) plus a thin `index_file_symbols(path)` Tauri command that reads the file and delegates. The frontend gets a hand-typed `listFileSymbols(path)` wrapper following the existing `invoke<T>(...)` pattern used by every current command. This is Plan 3 of the Ken IDE build (Milestone 1 — The Index); it is the seed the project-wide index (Plan 4) and persistence (Plan 5) grow from.

**Tech Stack:** Rust, `tree-sitter` 0.24, `tree-sitter-typescript` 0.23, `serde`, Tauri commands, TypeScript + `@tauri-apps/api/core`.

**Scope discipline (locked decisions):**
- **No `tauri-specta`.** Typed IPC is an all-or-nothing ~50-command migration and is its own future plan; this plan uses the established hand-typed `invoke<ReturnType>("cmd", {...})` pattern.
- **TypeScript only.** Uses the `LANGUAGE_TYPESCRIPT` grammar; no `.tsx`/JSX, no language detection (that is Plan 4).
- **Per-file, synchronous, no caching/persistence.** Project-wide indexing, background work, and on-disk persistence are Plans 4 and 5. Parsing is CPU-bound and fast for one file, so the command is a plain sync `#[tauri::command]` — no `tokio` feature changes.
- **Six symbol kinds only** (function, class, method, interface, typeAlias, enum). Variables/exports/imports are deferred (YAGNI) — they belong with the dependency-graph work.

---

## File Structure

| File | Responsibility |
|---|---|
| `src-tauri/Cargo.toml` | Add `tree-sitter` + `tree-sitter-typescript` deps |
| `src-tauri/src/modules/mod.rs` | Register `pub mod index;` |
| `src-tauri/src/modules/index/mod.rs` | Module root: `pub mod symbols;` |
| `src-tauri/src/modules/index/symbols.rs` | `Symbol`, `SymbolKind`, pure `extract_symbols`, the `index_file_symbols` command, unit tests |
| `src-tauri/src/lib.rs` | Register `index::symbols::index_file_symbols` in `generate_handler!` |
| `src-tauri/tests/index_symbols.rs` | Integration test: write a temp `.ts` file, call the command, assert symbols |
| `src/modules/index/symbols.ts` | Hand-typed `Symbol`/`SymbolKind` + `listFileSymbols(path)` wrapper |
| `src/modules/index/index.ts` | Barrel re-export |

---

### Task 1: Add tree-sitter dependencies and the `index` module skeleton

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/modules/mod.rs`
- Create: `src-tauri/src/modules/index/mod.rs`
- Create: `src-tauri/src/modules/index/symbols.rs`

- [ ] **Step 1: Add the dependencies to `src-tauri/Cargo.toml`**

In the `[dependencies]` section (after the `notify = "8.2.0"` line), add:
```toml
tree-sitter = "0.24"
tree-sitter-typescript = "0.23"
```

- [ ] **Step 2: Create the module root `src-tauri/src/modules/index/mod.rs`**

```rust
pub mod symbols;
```

- [ ] **Step 3: Create `src-tauri/src/modules/index/symbols.rs` with the types and a stub**

```rust
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SymbolKind {
    Function,
    Class,
    Method,
    Interface,
    TypeAlias,
    Enum,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: usize,
    pub end_line: usize,
}

pub fn extract_symbols(_source: &str) -> Vec<Symbol> {
    Vec::new()
}
```

- [ ] **Step 4: Register the module in `src-tauri/src/modules/mod.rs`**

Open `src-tauri/src/modules/mod.rs` and add `pub mod index;` alongside the other `pub mod` declarations (keep the file's existing alphabetical/grouped ordering — place it so the list stays readable, e.g. before `pub mod net;`).

- [ ] **Step 5: Build to confirm the dependencies resolve and compile**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: compiles successfully (tree-sitter + grammar download and build). If cargo reports a version conflict between `tree-sitter` and `tree-sitter-typescript`, pin `tree-sitter` to the exact version the grammar crate requires (the error message names it) and re-run — this is a real resolution step, not a guess.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/modules/mod.rs src-tauri/src/modules/index/
git commit -m "feat(index): add tree-sitter deps and index module skeleton"
```

---

### Task 2: Extract functions and classes (TDD)

**Files:**
- Modify: `src-tauri/src/modules/index/symbols.rs`

- [ ] **Step 1: Write the failing test**

Append to `src-tauri/src/modules/index/symbols.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_functions_and_classes() {
        let src = "function greet(name: string) {\n  return name;\n}\n\nclass Service {\n}\n";
        let symbols = extract_symbols(src);
        assert_eq!(
            symbols,
            vec![
                Symbol {
                    name: "greet".to_string(),
                    kind: SymbolKind::Function,
                    start_line: 1,
                    end_line: 3,
                },
                Symbol {
                    name: "Service".to_string(),
                    kind: SymbolKind::Class,
                    start_line: 5,
                    end_line: 6,
                },
            ]
        );
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib index::symbols::tests::extracts_functions_and_classes`
Expected: FAIL — `extract_symbols` returns an empty vec, so `assert_eq!` panics with a left/right mismatch.

- [ ] **Step 3: Implement parsing and the node walk**

Replace the stub `extract_symbols` in `src-tauri/src/modules/index/symbols.rs` with:
```rust
use tree_sitter::{Node, Parser};

fn node_name(node: Node, source: &[u8]) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    name_node
        .utf8_text(source)
        .ok()
        .map(|s| s.to_string())
}

fn kind_for(node_kind: &str) -> Option<SymbolKind> {
    match node_kind {
        "function_declaration" => Some(SymbolKind::Function),
        "class_declaration" => Some(SymbolKind::Class),
        _ => None,
    }
}

fn walk(node: Node, source: &[u8], out: &mut Vec<Symbol>) {
    if let Some(kind) = kind_for(node.kind()) {
        if let Some(name) = node_name(node, source) {
            out.push(Symbol {
                name,
                kind,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, out);
    }
}

pub fn extract_symbols(source: &str) -> Vec<Symbol> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    walk(tree.root_node(), bytes, &mut out);
    out
}
```

> Note: delete the old stub `pub fn extract_symbols(_source: &str) -> Vec<Symbol> { Vec::new() }` from Task 1 — it is fully replaced by the version above. Keep a single `use serde::Serialize;` at the top; the new `use tree_sitter::{Node, Parser};` goes with it.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib index::symbols::tests::extracts_functions_and_classes`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/modules/index/symbols.rs
git commit -m "feat(index): extract function and class symbols via tree-sitter"
```

---

### Task 3: Extract methods, interfaces, type aliases, and enums (TDD)

**Files:**
- Modify: `src-tauri/src/modules/index/symbols.rs`

- [ ] **Step 1: Write the failing test**

Add a second test inside the existing `mod tests` block in `src-tauri/src/modules/index/symbols.rs`:
```rust
    #[test]
    fn extracts_methods_interfaces_type_aliases_and_enums() {
        let src = concat!(
            "interface Repo {\n",
            "  id: number;\n",
            "}\n",
            "type Id = string;\n",
            "enum Color {\n",
            "  Red,\n",
            "}\n",
            "class Store {\n",
            "  save() {}\n",
            "}\n",
        );
        let symbols = extract_symbols(src);
        let pairs: Vec<(&str, &SymbolKind)> =
            symbols.iter().map(|s| (s.name.as_str(), &s.kind)).collect();
        assert_eq!(
            pairs,
            vec![
                ("Repo", &SymbolKind::Interface),
                ("Id", &SymbolKind::TypeAlias),
                ("Color", &SymbolKind::Enum),
                ("Store", &SymbolKind::Class),
                ("save", &SymbolKind::Method),
            ]
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib index::symbols::tests::extracts_methods_interfaces_type_aliases_and_enums`
Expected: FAIL — `kind_for` only maps functions/classes, so interfaces/type aliases/enums/methods are missing from the result.

- [ ] **Step 3: Extend `kind_for` with the remaining node kinds**

In `src-tauri/src/modules/index/symbols.rs`, replace the `kind_for` function body with the full mapping:
```rust
fn kind_for(node_kind: &str) -> Option<SymbolKind> {
    match node_kind {
        "function_declaration" => Some(SymbolKind::Function),
        "class_declaration" => Some(SymbolKind::Class),
        "method_definition" => Some(SymbolKind::Method),
        "interface_declaration" => Some(SymbolKind::Interface),
        "type_alias_declaration" => Some(SymbolKind::TypeAlias),
        "enum_declaration" => Some(SymbolKind::Enum),
        _ => None,
    }
}
```

- [ ] **Step 4: Run both tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib index::symbols`
Expected: PASS (both `extracts_functions_and_classes` and `extracts_methods_interfaces_type_aliases_and_enums`).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/modules/index/symbols.rs
git commit -m "feat(index): extract method, interface, type-alias, and enum symbols"
```

---

### Task 4: Add the Tauri command, register it, and the typed frontend wrapper

**Files:**
- Modify: `src-tauri/src/modules/index/symbols.rs`
- Modify: `src-tauri/src/lib.rs`
- Create: `src-tauri/tests/index_symbols.rs`
- Create: `src/modules/index/symbols.ts`
- Create: `src/modules/index/index.ts`

- [ ] **Step 1: Add the command to `src-tauri/src/modules/index/symbols.rs`**

Add this above the `#[cfg(test)]` block (and after `extract_symbols`):
```rust
#[tauri::command]
pub fn index_file_symbols(path: String) -> Result<Vec<Symbol>, String> {
    let source = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    Ok(extract_symbols(&source))
}
```

- [ ] **Step 2: Register the command in `src-tauri/src/lib.rs`**

In `src-tauri/src/lib.rs`, add `index` to the `use modules::{...}` import on line 3 (keep it grouped/ordered, e.g. `use modules::{agent, fs, git, history, index, net, pty, secrets, shell, workspace};`). Then inside the `tauri::generate_handler![ ... ]` list, add a line (e.g. right after the `history::history_list,` entry):
```rust
            index::symbols::index_file_symbols,
```

- [ ] **Step 3: Write the integration test `src-tauri/tests/index_symbols.rs`**

```rust
use std::io::Write;

use terax_lib::modules::index::symbols::{index_file_symbols, SymbolKind};

#[test]
fn command_returns_symbols_for_a_file() {
    let mut file = tempfile::Builder::new()
        .suffix(".ts")
        .tempfile()
        .expect("create temp file");
    write!(file, "export function run() {{}}\nclass Engine {{}}\n").expect("write source");

    let path = file.path().to_string_lossy().to_string();
    let symbols = index_file_symbols(path).expect("command should succeed");

    let pairs: Vec<(&str, &SymbolKind)> =
        symbols.iter().map(|s| (s.name.as_str(), &s.kind)).collect();
    assert_eq!(
        pairs,
        vec![
            ("run", &SymbolKind::Function),
            ("Engine", &SymbolKind::Class),
        ]
    );
}

#[test]
fn command_errors_on_missing_file() {
    let result = index_file_symbols("/no/such/ken/file.ts".to_string());
    assert!(result.is_err());
}
```

- [ ] **Step 4: Run the Rust test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS — including `command_returns_symbols_for_a_file` and `command_errors_on_missing_file`, with no regressions in existing tests.

- [ ] **Step 5: Create the typed frontend wrapper `src/modules/index/symbols.ts`**

```ts
import { invoke } from "@tauri-apps/api/core";

export type SymbolKind =
  | "function"
  | "class"
  | "method"
  | "interface"
  | "typeAlias"
  | "enum";

export interface FileSymbol {
  name: string;
  kind: SymbolKind;
  startLine: number;
  endLine: number;
}

export function listFileSymbols(path: string): Promise<FileSymbol[]> {
  return invoke<FileSymbol[]>("index_file_symbols", { path });
}
```

> Note: the Rust `Symbol` uses `#[serde(rename_all = "camelCase")]` on `SymbolKind` (so e.g. `TypeAlias` → `"typeAlias"`), and serde serializes the snake_case struct fields `start_line`/`end_line`. Tauri's command layer does NOT auto-camelCase struct field names, so the TS `FileSymbol` interface here intentionally mirrors the Rust field casing. **Verify this in Step 7** by checking the actual JSON shape — if the fields arrive as `start_line`/`end_line`, update the interface to match (this is the one cross-boundary contract to confirm by hand, exactly the drift `tauri-specta` would later remove).

- [ ] **Step 6: Create the barrel `src/modules/index/index.ts`**

```ts
export { listFileSymbols } from "./symbols";
export type { FileSymbol, SymbolKind } from "./symbols";
```

- [ ] **Step 7: Verify the frontend typechecks and confirm the JSON field casing**

Run: `pnpm check-types`
Expected: no type errors.

To confirm the serialized field names match the `FileSymbol` interface, inspect how serde serializes the struct: the `Symbol` fields are declared `start_line`/`end_line` with no field-level rename, so the JSON keys are `start_line`/`end_line`. **Update `src/modules/index/symbols.ts` so the interface fields are `start_line` and `end_line`** (snake_case) to match the wire format, and keep `name` and `kind`. (If you prefer camelCase on the wire, instead add `#[serde(rename_all = "camelCase")]` to the `Symbol` struct in `symbols.rs` — pick ONE and make both sides agree. Default for this plan: snake_case fields on both sides, since that is the lower-risk single-source change.)

Re-run: `pnpm check-types`
Expected: no type errors.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/modules/index/symbols.rs src-tauri/src/lib.rs src-tauri/tests/index_symbols.rs src/modules/index/
git commit -m "feat(index): index_file_symbols command and typed frontend wrapper"
```

---

## Self-Review

**Spec coverage (against `ken-ide-implementation-plans.md` Plan 3 — "Parse a file and emit symbols in Rust. Done when: a command returns the symbol list for one file"):**
- Parse a file in Rust: Tasks 1–3 (tree-sitter + `extract_symbols`). ✓
- Emit symbols: `Symbol`/`SymbolKind` serde types, six kinds. ✓
- A command returns the symbol list for one file: `index_file_symbols` (Task 4) + registration + integration test proving it. ✓
- Consumable by the frontend (for Plan 6 navigation): typed `listFileSymbols` wrapper. ✓
- Against design spec §5.3 (indexer scope = project graph / symbols for search, NOT LSP correctness): this slice is the per-file symbol primitive the project index (Plan 4) will fan out over. No overreach into LSP territory. ✓

**Placeholder scan:** No "TBD"/"handle edge cases"/"similar to Task N". Every code step shows complete code. The one genuine verification step (JSON field casing, Task 4 Step 7) is a concrete check with a stated default resolution, not a placeholder. ✓

**Type consistency:** `Symbol { name, kind, start_line, end_line }` and `SymbolKind { Function, Class, Method, Interface, TypeAlias, Enum }` are defined once (Task 1) and used identically in Tasks 2–4. `kind_for` node-kind strings (`function_declaration`, `class_declaration`, `method_definition`, `interface_declaration`, `type_alias_declaration`, `enum_declaration`) are tree-sitter-typescript grammar node names. The frontend `FileSymbol`/`SymbolKind` mirror the Rust types, with the field-casing contract pinned in Task 4 Step 7. The command name `index_file_symbols` matches between the Rust command, the `generate_handler!` registration, the integration test, and the `invoke("index_file_symbols", ...)` call. ✓

**Known risk flagged in-plan:** exact `tree-sitter` / `tree-sitter-typescript` version compatibility (Task 1 Step 5 has a pin-and-retry instruction), and the cross-boundary field casing (Task 4 Step 7). Both have concrete resolutions.
