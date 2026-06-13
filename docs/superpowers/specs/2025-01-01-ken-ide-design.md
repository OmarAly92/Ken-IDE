# Ken IDE - Design Document

## 1. Vision

Ken IDE is an AI-native desktop IDE inspired by the JetBrains experience. The goal is not to create another VS Code clone, but to build a complete development environment that feels cohesive, project-aware, keyboard-first, and deeply integrated with AI workflows.

**Core Differentiation:** Maximum performance and instant feel - the advantage over JetBrains IDEs which are slow and bloated.

### Priorities (in order)

1. Super fast
2. Small size
3. JetBrains feel & UI (explicitly NOT VS Code)
4. Deep AI integration
5. JetBrains keymap import
6. Strict split: Rust owns ALL logic; React is UI only and calls Rust via Tauri commands

---

## 2. Core Principles

### Project-Centric
The IDE understands the entire project - files, classes, functions, dependencies, git history, database schemas, and project architecture.

### AI-Native
AI integrated into the development workflow - explain architecture, generate features, refactor across workspace, detect dead code, generate tests, analyze performance.

### Keyboard First
Majority of IDE actions accessible without mouse - Search Everywhere, Go To Definition, Find References, Quick Fixes, Refactoring, AI Actions.

### Fast and Lightweight
- Fast startup
- Low memory usage
- Native desktop performance
- Efficient indexing

**Competitive Advantage:** Instant feel - all operations feel immediate, unlike JetBrains sluggishness.

### Local First (Settings & Project)
- Local settings
- Offline project management

"Local first" refers to settings and project management working fully offline. It does **not** mean local-only AI - see AI Layer for the provider stance.

---

## 3. Architecture

### Split: Rust Backend + React UI

| Layer | Responsibility |
|-------|----------------|
| **Rust Backend** | Project indexing, symbol extraction, file search, git operations, terminal PTY, LSP/DAP management, AI proxy |
| **React UI** | Rendering, user input, state coordination, panels layout |

**Philosophy:** All "thinking" and heavy processing in Rust. React is a lightweight display layer that calls Rust via Tauri commands.

### The Rust / UI Boundary

```
Rust (all logic)              React (UI only)
─────────────────             ──────────────────────────────
indexing, search, git, AI   → call via Tauri, display results
LSP / DAP orchestration     → show completions / diagnostics
syntax highlighting (later) → render tokens as decorations
                              + cursor / selection / typing / scroll
                                (MUST stay local - <16ms frame budget)
```

**Hard limit:** keystrokes, cursor, selection, and scrolling never round-trip to Rust. Rust answers everything else asynchronously.

---

## 4. Technology Stack

| Concern | Choice |
|---------|--------|
| Desktop framework | **Tauri** - cross-platform, native performance, small bundle |
| Backend | **Rust** - FS ops, indexing, git, processes, terminal, LSP/DAP, AI tooling |
| Frontend | **React + TypeScript** |
| State management | **Zustand** - global state, workspace, UI, settings |
| Editor | **CodeMirror 6** (see Editor Architecture) |
| Terminal | **xterm.js** with WebGL renderer |
| AI | **Vercel AI SDK** (see AI Layer) |
| Index persistence | **Memory-mapped binary** - fastest for index persistence |
| Git | Rust-based git operations |

---

## 5. Subsystems

### 5.1 Editor Architecture

CodeMirror 6 runs in the webview (React side) - there is no Rust editor component. The design draws a hard line between what *must* stay local for responsiveness and what Rust owns.

**Decision: CodeMirror 6** (not Monaco). Lean bundle (hundreds of KB), modular, thin frontend. Lets Rust own the maximum logic (highlighting, LSP orchestration) while React/CM stays a thin UI. No VS Code DNA - the JetBrains feel is built in our own shell, not inherited from the editor. Monaco was the alternative (faster to wire LSP day one) but its heavy bundle, frontend-resident language services, and VS Code feel conflict with priorities 2, 3, and 6. Tradeoff accepted: the LSP↔editor bridge is built by hand in Phase 3 (well-understood work).

#### Highlighting Strategy
- **Start with CodeMirror's built-in Lezer grammars** - fast, free, in-bundle. Good enough for Phases 1-2.
- **Move to Rust tree-sitter only if needed** - i.e. when the project index already requires tree-sitter parsing, reuse those tokens for highlighting. Do not build two highlighting paths early.

#### LSP Bridge (Phase 3)
The single largest editor task. Built by hand (CM6 LSP patterns are well-trodden):
- Map LSP completions → CM completion source + custom React popup
- Map LSP diagnostics → CM decorations + gutter markers
- Map LSP hovers / signature help → CM tooltips
- Quick fixes / intentions → gutter lightbulb + action menu

#### JetBrains Feel Lives in the Shell, Not the Editor
The JetBrains feel is ~90% behavior and UI polish built *on top* of the editor: completion popup styling, peek definition, inline hints, smart selection-expand, Search Everywhere overlays, panel animations, status bar. These are built in our React shell + backed by fast Rust responses - independent of the editor engine choice.

### 5.2 LSP Integration

#### Auto-Detect + Override in Settings
- Auto-detect languages from project files (package.json, Cargo.toml, go.mod)
- Proactive start for common languages (TypeScript, Rust, Python)
- Lazy-load others on first use
- Settings panel to override LSP path per language

#### Implementation
```
React UI  →  Tauri invoke("lsp_request")  →  LspManager  →  LSP Server
              ↑                                       ↓
          JSON response  ←  ←  ←  ←  ←  ←  ←  ←
```

#### LSP Manager Structure (Rust)
```rust
struct LspManager {
    project_root: PathBuf,
    servers: HashMap<Language, LspServer>,
    request_tx: HashMap<Language, mpsc::Sender<LspRequest>>,
    response_tx: broadcast::Sender<LspResponse>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<LspResponse>>>>,
    next_id: Arc<AtomicU64>,
}
```

**Async multiple requests** - non-blocking, multiple in-flight simultaneously.

### 5.3 Indexing System

#### Scope: Project Graph, Not Language Correctness
The indexer does **not** compete with LSP. LSP owns per-file language correctness (go-to-definition, find-references, completions, diagnostics). The indexer owns project-wide speed and AI context - the things LSP can't or won't deliver fast and offline.

| Owner | Responsibility |
|-------|----------------|
| **LSP** | Per-file symbols, go-to-definition, find-references, completions, diagnostics |
| **Indexer** | Cross-file project graph (imports, dependency edges, dead-code candidates), instant fuzzy file/symbol search (Search Everywhere), framework-aware mapping (routes, components, schemas), AI context |

**Overlap rule:** When both can answer (e.g. symbol lookup), use the index for the instant first paint, then reconcile with LSP for precision.

#### Data to Index
- Files (path, name, extension)
- Symbols (for fuzzy search and project graph - not a replacement for LSP symbol resolution)
- Imports/exports (dependency edges)
- Dependencies
- Routes/components

#### Timing
- Project open → Full index in background (show progress)
- User works → Incremental index on file save
- Idle → Deep indexing

#### Storage: Memory-Mapped Binary
- Hot data (search results, goto): in-memory BTreeMap
- Cold data (persist): memory-mapped binary file
- Load from disk on project open, update in memory

**Crash safety:**
- Write to temp file, then atomic rename
- Checksums to detect corruption
- Version header for migrations

### 5.4 Terminal Architecture

#### Structure
- Tabs as top-level containers
- Splits inside tabs
- Multiple sessions managed in Rust

#### Features
- Auto-detect shell ($SHELL env, fallback to /bin/bash or /bin/zsh)
- Settings override for shell path
- Kill process when terminal closes

#### Rust PTY Manager
```rust
struct PtyManager {
    sessions: HashMap<TerminalId, PtySession>,
    next_id: AtomicU64,
}

struct PtySession {
    process: Child,
    pty: PtyMaster,
    cwd: PathBuf,
}
```

### 5.5 AI Layer

#### Providers
- **Vercel AI SDK** - one interface across all providers.
- **Provider-agnostic by design** - local (Ollama, LM Studio) and remote (OpenAI, Anthropic, Gemini, OpenRouter) are both first-class. The user chooses. This is **not** local-only.
- **Default provider (first-run):** Cloud. A new user adds an API key and gets working AI instantly on any machine - no model download or hardware requirement. Local providers are opt-in for offline/privacy use.
- **Foundation:** reuse Terax's existing AI module (provider abstraction, OS-keychain keyring, tool harness, approval flow, redaction/security). Ken IDE adds **IDE-aware tools** on top of that substrate.

#### Thesis: Beat Cursor on Context Accuracy

Cursor's AI works off **text** - it reads files and guesses relationships via embeddings. Ken IDE has primitives Cursor lacks: a **resolved symbol/dependency graph**, **live LSP**, a **debugger**, and **git history**. The strategy is to expose each as an **AI tool that returns verified facts**, so the model reasons over ground truth instead of guessing.

> "Cursor reads your code. Ken IDE understands it - and lets the AI ask the same questions a senior engineer's IDE answers instantly."

The wedge is **context accuracy**. Everything below serves it.

#### Tool Catalog (IDE primitive → AI tool)

**Graph / index tools** - auto-seeded into the prompt *and* callable on demand. Backed by the Rust index (microsecond lookups, no embeddings).

| AI tool | Returns |
|---|---|
| `get_definition(symbol)` | Exact def site + signature |
| `find_callers(symbol)` | Every real call site |
| `get_impls(trait/interface)` | All implementors |
| `get_type(expr)` | Resolved type |
| `get_dependents(file)` | Reverse-dependency edges (blast radius) |
| `call_hierarchy(fn)` | Up/down call tree |
| `neighborhood(symbol)` | The context slice: def + callers + callees + types + tests |

**`neighborhood(symbol)` is the killer primitive** - one call returns the precise context a question needs. Because it is a graph slice rather than whole files, more *relevant* code fits in *fewer* tokens → cheaper, faster, and more accurate than Cursor's "shovel in similar-looking chunks."

**Context mechanism:** hybrid - Ken auto-seeds the relevant `neighborhood()` slice instantly (no model round-trip), and the AI can call the graph tools above to expand precisely on demand.

**Two precision tiers - do not present these tools as uniform:**

| Tier | Tools | Source | Available |
|---|---|---|---|
| **Approximate (instant)** | `get_definition`, `find_callers`, `neighborhood()` seed, fuzzy search | tree-sitter / index - name-based | Phase 2 |
| **Precise (semantic)** | `get_type`, `get_impls`, exact `find_references`, rename, code-actions, diagnostics | LSP - type-resolved | Phase 3 |

So `neighborhood()` ships in two versions: **v1** tree-sitter-approximate (Phase 2), **v2** LSP-reconciled-precise (Phase 3). Do not promise resolved-type accuracy before LSP exists.

#### LSP as a Verification & Action Layer - "AI proposes, the language server proves"

- **Verified edits:** before an AI edit is *ever shown* to the user, apply it to a scratch buffer and run LSP/compiler diagnostics. If it doesn't typecheck, feed the errors back and let the model self-correct in a loop. **Edits that reach the user already compile.**
- **Deterministic refactors:** when the AI wants to rename/move a symbol, it calls the **LSP rename / code-action** tool instead of hand-editing text. A 200-file rename is one guaranteed-correct operation, not 200 fragile string edits.
- `get_diagnostics(file)` and `apply_quick_fix(diagnostic)` - the AI sees real errors and picks from real, language-server-provided fixes.

#### Debugger (DAP) as Context

- `get_runtime_state()` → actual variable values, call stack, resolved types *at runtime* when stopped at a breakpoint.
- "Why is this null?" is answered from the real stack frame, not speculation.
- AI-driven debugging: AI sets breakpoints, runs, inspects, forms a hypothesis. A genuine, defensible gap vs Cursor. (Depends on Phase 4 DAP work.)

#### Git History as Context

- `blame(lines)` + `recent_changes(symbol)` → the AI answers "why is this code like this?" from commit messages and diffs, and avoids re-introducing reverted bugs.

#### Runtime / Terminal as Context

- `run_tests_for(symbol)` - use the graph to map a changed symbol to *exactly* its tests, run only those, read real output. Tight verify loop instead of running everything.

#### Build Sequence

1. **Graph-tool layer** (the wedge) - auto-seed `neighborhood()` + expose graph tools. Reusable by every other feature.
2. **Verified-edit loop** - makes accurate context actually land as working code; the trust multiplier.
3. **Deterministic LSP refactors** - high wow-factor, low model risk.
4. **Debugger context** - unique, but gated on Phase 4 DAP.

#### Privacy

Cloud is the default, so code leaves the machine on cloud providers. The graph-slice approach sends *less* code (precise slices, not whole files). Local providers remain a first-class opt-in for fully-offline/private use. (Detailed privacy controls - redaction, per-project policy - tracked in Open Questions.)

### 5.6 Git Integration

- Rust-based git operations.
- Surfaced as a tool window.

> Note: scope (diff view, blame, staging, commit, branch, merge-conflict resolution) is **not yet designed** - see Open Questions.

---

## 6. UI & Experience

### 6.1 Layout (Simplified JetBrains-like)

```
┌────────────────────────────────────────────────────────────┐
│ [Search: Ctrl+Shift+Shift]                         [≡][□] │
├─────┬────────────────────────────────────────────────────┤
│     │ [Tab Bar]  App.tsx  │ Button.tsx  │         [×]   │
│  📁 ├────────────────────────────────────────────────────┤
│  ▼  │                                                    │
│ src │              Editor                                │
│  ├─ │                                                    │
│  ├─ │                                                    │
│     ├────────────────────────────────────────────────────┤
│     │ Terminal                                    [▼][×] │
├─────┴────────────────────────────────────────────────────┤
│ main │ TypeScript │ Ln 45, Col 12                      │
└────────────────────────────────────────────────────────────┘
```

### 6.2 JetBrains "Feeling"
- **Panel animations** - smooth slide in/out (200-300ms, ease-out)
- **Search everywhere** - instant results
- **Tool windows** - quick access, keyboard toggleable
- **Keyboard-first** - all actions via shortcuts
- **Status feedback** - always know what's happening
- **Tab switching** - instant switch

### 6.3 Tool Windows
- Project (tree view)
- Terminal
- Git
- Debug
- AI Assistant

Toggle: Ctrl+1, Ctrl+2, Ctrl+3, etc.

### 6.4 Keyboard Shortcuts (JetBrains Keymap)

| Action | Shortcut |
|--------|----------|
| Search everywhere | Shift+Shift |
| Go to file | Ctrl+Shift+N |
| Go to class | Ctrl+N |
| Go to symbol | Ctrl+Shift+Alt+N |
| Recent files | Ctrl+E |
| Find in files | Ctrl+Shift+F |
| Find | Ctrl+F |
| Replace | Ctrl+R |
| Command palette | Ctrl+Shift+P |
| Toggle tool window 1-4 | Ctrl+1 … Ctrl+4 |
| Close tab | Ctrl+F4 |
| Settings | Ctrl+Alt+S |

#### Import JetBrains Keymaps
- Store as JSON
- Map JetBrains actions to internal actions
- Allow user to import from file

### 6.5 Themes

**Built-in:** IntelliJ Light, Darcula, One Dark, other popular IDE themes.
**Custom:** user creates own themes; import/export theme files; reuse current Ken-IDE themes.

```typescript
interface Theme {
  name: string;
  colors: {
    background: string;
    foreground: string;
    accent: string;
  };
  editor: EditorTheme;
  terminal: TerminalTheme;
}
```

---

## 7. Roadmap

### 7.1 Development Phases

| Phase | Scope |
|-------|-------|
| **1** | Core IDE (editor + terminal + file tree); make it fast and stable |
| **2** | Project awareness (indexing, navigation); JetBrains-like experience |
| **3** | LSP integration, language support, diagnostics, refactoring |
| **4** | DAP integration, debugging support |
| **5** | AI-native workflows, workspace-level AI actions |
| **6** | Database tools, schema explorer, query editor |
| **7** | Open the existing extension API to third parties (marketplace, sandboxing, distribution) |

**Extensibility note:** Extensibility is an *architecture* decision, not a Phase 7 feature. From Phase 1, build core features (git, AI, debug, tool windows) as consumers of an internal extension API - the same API a third party will eventually use. This forces clean seams. Phase 7 is opening that API to the public (the business), not inventing extensibility (the tech). If your own tool windows aren't already plugins, the API is wrong.

### 7.2 Phase Gates

Each phase ends with a checklist that **must pass before the next begins**. The biggest risk for a small-team IDE is going wide and shallow - gates keep it deep.

- **Phase 1 gate:** Performance budget below is green in CI. Editor + terminal + file tree stable.
- **Phase 2 gate:** Search Everywhere < 50ms on a 10k-file project. Project graph index passing.
- **Phase 3 gate:** LSP diagnostics/navigation working for TypeScript, Rust, Python without regressing the perf budget.
- **Phase 4 gate:** Debugger stops, steps, inspects on at least one language end-to-end.
- **Phase 5 gate:** AI workflow works against both a local and a remote provider.

**Rule:** Do not start the next phase until the current gate passes.

### 7.3 Performance Targets

These are CI gates, not aspirations. A PR that regresses any budget fails. (Tune baselines to reference hardware.)

| Metric | Target |
|--------|--------|
| Cold start to interactive window | < 800ms |
| Open project (10k files) → tree usable | < 1s |
| Full background index (10k files) | < 5s |
| Search Everywhere keystroke → results | < 50ms |
| Tab switch / file open (cached) | < 16ms (one frame) |
| Idle memory (medium project) | < 300MB |

**Key metric:** All operations feel immediate - this is the competitive advantage over JetBrains. The advantage erodes silently without a regression test, so every target above is enforced in CI.

---

## 8. Open Questions / Not Yet Designed

Genuinely undecided items. (Several earlier gaps - secret storage, file watching, find-in-files, fuzzy search, auto-update, session restore, diff/diagnostics UI - are already solved by the cloned codebase; see §9.)

- **AI privacy controls** - the AI Layer is designed (§5.5), but the detailed privacy story remains open: redaction rules, per-project send policies, and what telemetry/logging (if any) the AI surface keeps. Inline-completion latency budgets also TBD.
- **Settings/config system** - referenced throughout (LSP path, shell, themes, keymap) but never defined: format, global vs per-project, schema, live reload.
- **Cross-platform keybindings** - keymap uses Ctrl everywhere, but JetBrains on macOS uses Cmd. Need a platform-aware mapping.
- **DAP / debugging architecture** - Phase 4 and a tool window, but no design like LSP has.
- **Git library decision** - no git crate today (source control shells out to `git`). Decide `gix` vs continued shell-out for IDE-grade git + AI `blame()`/`recent_changes()` tools (see §9).
- **Testing strategy** - Rust unit, integration, and E2E; how the perf gates run in CI.
- **Large/binary file handling** - huge files, binaries, images; encoding and line-ending detection.

---

## 9. Implementation Prerequisites

Ken IDE is built on a clone of Terax. This section separates what the clone already provides from what must be added before/early in implementation.

### Already provided by the clone (do not re-add)

| Capability | Dependency in repo |
|---|---|
| Secure secret storage | `keyring` 3.6 (apple-native / windows-native) |
| File watching | `notify` 8.2 |
| Find in Files (ripgrep internals) | `ignore`, `grep-searcher`, `grep-regex`, `globset` |
| Fuzzy matching (Search Everywhere) | `nucleo-matcher` |
| Terminal PTY | `portable-pty` |
| Auto-update | `tauri-plugin-updater` |
| Session / layout restore | `tauri-plugin-window-state`, `tauri-plugin-store`, existing "spaces" work |
| AI providers + harness | `@ai-sdk/*`, `ai` v6, `zod` tool schemas, `tokenlens`, `src/modules/ai/tools/*` |
| AI edit diff UI | `@codemirror/merge` |
| Diagnostics UI | `@codemirror/lint` |
| Command palette UI | `cmdk` |
| Bundle-size gate | `size-limit` |

The new AI graph tools plug into the **existing** AI tool harness (`src/modules/ai/tools/*`), not a new system.

### Must add before / early in implementation

| Need | Recommendation | Notes |
|---|---|---|
| Parsing / symbol extraction | `tree-sitter` + grammar crates | Backbone of the index. Nothing parses code in Rust today (CM Lezer is frontend-only). |
| Index persistence | `rkyv` + `memmap2` | rkyv = zero-copy deserialization, matches the "mmap, microsecond load" goal. `bincode` is the simpler fallback. |
| LSP client | `lsp-types` + JSON-RPC over stdio | **`tokio` is currently `features=["rt"]` only** - add `process`, `io-util`, `sync`, likely `rt-multi-thread` for the `LspManager` design. |
| Typed IPC | `tauri-specta` + `specta` | Highest-leverage add. Auto-generates typed TS bindings for every command/event - enforces the "React calls Rust for everything" boundary and kills drift bugs. |
| Git library | decide `gix` vs shell-out | No git crate today (source-control shells out to `git`). `gix` (pure Rust, fast, fits "small") is the clean path for AI `blame()`/`recent_changes()`; shell-out is lighter on deps. |
| Perf benchmarking | `criterion` + startup/mem script | CI perf gates (§7.3) are fiction without a harness. `size-limit` already covers bundle. |
| DAP (Phase 4) | `dap-types` | Defer; same JSON-over-stdio shape as LSP. |

### Highest-leverage pre-implementation decisions

1. **Typed IPC contract via `tauri-specta`** - lock the command/event generation pipeline before writing commands; it is the central interface of the architecture.
2. **tree-sitter as the single parse layer** - one parser feeds the index/graph *and* (later) editor highlighting. Do not build two parse paths.
3. **Index schema before AI graph tools** - the graph shape (symbols, edges) is consumed by search, navigation, and the AI catalog. Get it right once; everything reads from it.
