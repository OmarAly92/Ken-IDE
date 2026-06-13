# Ken IDE - Implementation Plan Breakdown

How the design spec (`2025-01-01-ken-ide-design.md`) is decomposed into multiple small implementation plans. One monolithic plan for an IDE would be unreviewable and wrong - instead we use a sequence of small vertical slices, where each plan ends in something you can actually run and see.

## The Slicing Principle

Each plan must satisfy four rules:

1. **Demoable** - ends in something you can click/run, not "infrastructure that's half-wired."
2. **Vertical** - cuts through Rust → IPC → React so a feature works end-to-end, rather than "all the backend, then all the frontend."
3. **Dependency-ordered** - each builds on the last; no forward references.
4. **Gate-aligned** - clusters map to the phase gates in §7.2 of the design spec.

> The clone (Terax) already provides Phase 1 (editor, terminal, file tree). We start at reshaping + the index, not from zero.

---

## The Plan Sequence

### Milestone 0 - Foundation
Infrastructure, but each has a concrete done-state.

| Plan | Goal | Done when |
|------|------|-----------|
| **1 - Typed IPC + perf harness** | Wire `tauri-specta` so every command/event has generated TS types; stand up `criterion` + a startup/memory script in CI with a green baseline. | A sample command round-trips with generated types and the perf gate runs in CI. |
| **2 - Rebrand & IDE shell** | Terax → Ken IDE naming; reshape layout toward the §6.1 JetBrains shell (tool-window frame, status bar). | App boots as Ken IDE with the IDE chrome. |

### Milestone 1 - The Index
The wedge's foundation.

| Plan | Goal | Done when |
|------|------|-----------|
| **3 - tree-sitter parse + symbols (TypeScript only)** | Parse a file and emit symbols in Rust. | A command returns the symbol list for one file. |
| **4 - Project index + background indexing** | Index on project open with progress; query symbols project-wide. | Open a project, watch progress, query any symbol. |
| **5 - Index persistence (rkyv + memmap2)** | Atomic write, checksum, version header. | Reopen project loads index from disk fast (cold-start gate). |

### Milestone 2 - Navigation
First visible IDE value - Phase 2 gate.

| Plan | Goal | Done when |
|------|------|-----------|
| **6 - Search Everywhere / Go to File & Symbol** | `cmdk` + `nucleo` + index. | Shift+Shift gives instant fuzzy results <50ms on 10k files. |
| **7 - Graph edges + Go to Definition / Find Callers (approximate)** | Import/dependent edges; name-based navigation. | Jump to definition and list callers from the editor. |

### Milestone 3 - AI Wedge v1
The differentiator - ships before LSP.

| Plan | Goal | Done when |
|------|------|-----------|
| **8 - `neighborhood()` v1 + graph tools into the AI harness** | Auto-seed context; expose graph tools to the existing `src/modules/ai/tools/*`. | The AI answers a question using a precise graph slice instead of file dumps. |

### Milestone 4 - LSP
Phase 3 gate.

| Plan | Goal | Done when |
|------|------|-----------|
| **9 - LSP client + diagnostics (TypeScript)** | `LspManager` + `@codemirror/lint`. | Real red squiggles appear. |
| **10 - LSP navigation + completion** | Precise go-to-def, find-refs, hover, autocomplete. | Editor feels like an IDE. |
| **11 - Verified-edit loop + LSP refactors + `neighborhood()` v2** | "AI proposes, LSP proves"; deterministic rename/code-actions as AI tools. | An AI edit is rejected-and-retried because it didn't typecheck. |

### Milestone 5 - JetBrains Feel
Can interleave anytime after Plan 2.

| Plan | Goal | Done when |
|------|------|-----------|
| **12 - Keymap engine + JetBrains import + tool-window toggles + theme polish** | Platform-aware (Cmd/Ctrl) bindings. | Import a JetBrains keymap and toggle tool windows by shortcut. |

### Later (own spec → plan cycles)
- DAP / debugging (Phase 4)
- Database tools (Phase 6)
- Settings/config system (pull earlier if LSP-path/shell overrides need it)

---

## Where to Start

Plans 1 and 2 are independent and small - either is a clean first slice. Recommended: **Plan 1 first** - typed IPC is the architectural keystone; everything after rides on it.

Each plan gets its own `writing-plans` cycle: write it, execute, review, then write the next - never all at once.
