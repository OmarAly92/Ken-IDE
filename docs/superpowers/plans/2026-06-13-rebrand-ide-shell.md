# Rebrand & IDE Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the cloned Terax app to "Ken IDE" everywhere it is user-facing — app shell chrome (window/document titles, About, default theme), AI assistant identity (renamed to "Ken"), updater dialog copy, and settings copy — leaving internal wire identifiers intact, so the app boots and presents as Ken IDE with its existing JetBrains-style chrome.

**Naming convention:** the application/product is **"Ken IDE"**; the embedded AI assistant is **"Ken"**.

> **Scope note (added during execution):** This plan originally covered only the app shell chrome (Tasks 1–4). Final whole-branch review found ~24 additional user-facing "Terax" strings in the AI surfaces, updater dialog, and settings copy. Task 5 was folded in to cover them so the goal genuinely holds. The deb/rpm install-command filenames and the `TERAX.md` project-memory convention remain deliberately unchanged (see below + Known Follow-ups).

**Architecture:** This is Plan 2 of the Ken IDE build (see `docs/superpowers/specs/ken-ide-implementation-plans.md`). The cloned shell (`src/app/App.tsx`) already realizes the §6.1 layout — top Header+search, a left tool-window rail (`SidebarRail` → Explorer / Source Control), the center editor/terminal surface, and a bottom `StatusBar`. No structural reshape is required. This plan therefore does the rebrand (the real "boots as Ken IDE" deliverable) plus a small, bounded chrome-naming alignment (window-title fallback, default-theme display name, About panel). Deeper visual theming, the dockable tool-window framework, and Ctrl+1..N toggles are intentionally deferred to later plans (Plan 12 / themes work).

**Tech Stack:** Tauri 2 config (JSON), React + TypeScript, Vitest, Biome, Rust (build/clippy gate only — no Rust source changes).

---

## Brand Values (committed defaults — confirm at the review gate)

These exact strings are used throughout the tasks. If any is wrong, change it here and the tasks follow.

| Field | Value | Notes |
|---|---|---|
| Product / window title / display name | `Ken IDE` | user-facing |
| Bundle identifier | `app.omardev.ken` | changing from `app.crynta.terax` intentionally resets keychain / window-state / store data — correct for a fresh product |
| npm workspace name (`package.json`) | `ken-ide` | dev-facing only, no runtime impact |
| Publisher / copyright | `Crynta` (unchanged) | same owner |
| Short description | `AI-native IDE` | replaces "AI-native terminal emulator" |
| Long description | `Ken IDE — an AI-native desktop IDE with deep project awareness, integrated terminal, code editor, and AI agents.` | replaces the Terax terminal blurb |

### Deliberately NOT renamed (renaming these is a data migration or wire-protocol break, not a rename)

- Rust lib crate `terax_lib` and bin crate `terax` (`src-tauri/Cargo.toml`) — internal build identifiers referenced across `main.rs`, tests, and the Plan 1 bench. Per the project directive, keep unless trivially safe; they are not.
- `localStorage` key `terax-ui-theme-shadow` (`index.html`, `settings.html`) — renaming orphans the saved theme-shadow on every existing install.
- Theme **id** `terax-default` and the exported symbol `teraxDefault` — the id is persisted as the user's selected theme; only its *display* strings change.
- Tauri event names (`terax:settings-tab`, `terax:ai-attach-file`, `terax:agent-signal`) — backend/frontend contract.
- Shell-integration surface: `TERAX_TERMINAL` / `TERAX_BLOCKS` / `TERAX_USER_ZDOTDIR` env vars, `~/.cache/terax`, and the `notify;Terax;` PTY markers — a wire protocol baked into installed shell hooks.
- Updater endpoint, repo URL (`crynta/terax-ai`), and website (`terax.app`) — real infrastructure. Pointing them at non-existent Ken IDE infra would break auto-update. The About panel keeps these links until Ken IDE infra exists; see Task 3's note.
- Deb/rpm install-command filename templates (`UpdaterDialog.tsx:22,24`, e.g. `Terax_${version}_amd64.deb`) — these must match the actual Tauri bundle artifact name, which can only be confirmed by a real `pnpm tauri build`. Because the new `productName` "Ken IDE" contains a space (which affects artifact naming / shell-command quoting), this needs a deliberate fix, not a blind rename. See Known Follow-ups.
- `.terax-theme` theme file extension and the `readTeraxMd` / `TERAX.md` project-memory convention — file-format / file-name identifiers. Renaming them orphans existing exported themes and per-project memory files; treated like the storage keys above.

---

## File Structure

| File | Change |
|---|---|
| `src-tauri/tauri.conf.json` | Modify: productName, window title, identifier, bundle short/long description |
| `src-tauri/tauri.windows.conf.json` | Modify: window title |
| `package.json` | Modify: `name` |
| `index.html` | Modify: `<title>` |
| `settings.html` | Modify: `<title>` |
| `src/modules/tabs/lib/useWindowTitle.ts` | Modify: extract pure `computeWindowTitle`, set `APP_NAME` |
| `src/modules/tabs/lib/useWindowTitle.test.ts` | Create: unit tests for `computeWindowTitle` |
| `src/modules/theme/themes/terax-default.ts` | Modify: display `name` + `description` only (keep id + export symbol) |
| `src/settings/sections/AboutSection.tsx` | Modify: pre-load name fallback + displayed Bundle ID |

---

### Task 1: Rebrand Tauri config + npm package name

**Files:**
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/tauri.windows.conf.json`
- Modify: `package.json:2`

- [ ] **Step 1: Edit `src-tauri/tauri.conf.json` product identity**

Change line 3:
```json
  "productName": "Ken IDE",
```
Change the window title (line 16, inside `app.windows[0]`):
```json
        "title": "Ken IDE",
```
Change the identifier (line 5):
```json
  "identifier": "app.omardev.ken",
```

- [ ] **Step 2: Edit `src-tauri/tauri.conf.json` bundle descriptions**

Change the `bundle.shortDescription` and `bundle.longDescription` lines (currently lines 86-87):
```json
    "shortDescription": "AI-native IDE",
    "longDescription": "Ken IDE — an AI-native desktop IDE with deep project awareness, integrated terminal, code editor, and AI agents."
```
Leave `copyright`, `publisher`, `license`, `icon`, and the `updater` block unchanged.

- [ ] **Step 3: Edit `src-tauri/tauri.windows.conf.json` window title**

Change line 7:
```json
        "title": "Ken IDE",
```

- [ ] **Step 4: Edit `package.json` workspace name**

Change line 2:
```json
  "name": "ken-ide",
```

- [ ] **Step 5: Verify the configs are valid JSON and the Rust build is unaffected**

Run: `node -e "JSON.parse(require('fs').readFileSync('src-tauri/tauri.conf.json','utf8'));JSON.parse(require('fs').readFileSync('src-tauri/tauri.windows.conf.json','utf8'));JSON.parse(require('fs').readFileSync('package.json','utf8'));console.log('json ok')"`
Expected: `json ok`

Run: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings`
Expected: finishes with no warnings/errors (the bin crate is still `terax`; nothing in Rust references the identifier or productName).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/tauri.conf.json src-tauri/tauri.windows.conf.json package.json
git commit -m "feat(rebrand): rename app to Ken IDE in tauri config and package name"
```

---

### Task 2: Rebrand window title + HTML document titles

**Files:**
- Modify: `index.html:7`
- Modify: `settings.html:7`
- Modify: `src/modules/tabs/lib/useWindowTitle.ts`
- Test: `src/modules/tabs/lib/useWindowTitle.test.ts`

This task extracts the title-formatting logic into a pure function so the fallback name is unit-testable (the hook itself calls Tauri window APIs and can't be unit-tested directly).

- [ ] **Step 1: Write the failing test**

Create `src/modules/tabs/lib/useWindowTitle.test.ts`:
```ts
import { describe, expect, it } from "vitest";
import { APP_NAME, computeWindowTitle } from "./useWindowTitle";

describe("computeWindowTitle", () => {
  it("falls back to the app name when nothing else is available", () => {
    expect(computeWindowTitle("", "")).toBe("Ken IDE");
    expect(APP_NAME).toBe("Ken IDE");
  });

  it("shows the project alone when the tab label equals the project", () => {
    expect(computeWindowTitle("ken-ide", "ken-ide")).toBe("ken-ide");
  });

  it("shows the project alone when there is no tab label", () => {
    expect(computeWindowTitle("ken-ide", "")).toBe("ken-ide");
  });

  it("joins project and label when they differ", () => {
    expect(computeWindowTitle("ken-ide", "src")).toBe("ken-ide — src");
  });

  it("shows the label alone when there is no project", () => {
    expect(computeWindowTitle("", "Settings")).toBe("Settings");
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `pnpm test -- src/modules/tabs/lib/useWindowTitle.test.ts`
Expected: FAIL — `computeWindowTitle` / `APP_NAME` are not exported yet.

- [ ] **Step 3: Refactor `useWindowTitle.ts` to expose the pure function**

In `src/modules/tabs/lib/useWindowTitle.ts`, change the constant on line 6 and export it, add the pure helper, and have the hook use it. Replace:
```ts
const APP_NAME = "Terax";
```
with:
```ts
export const APP_NAME = "Ken IDE";

export function computeWindowTitle(
  project: string,
  label: string,
  appName: string = APP_NAME,
): string {
  if (project && label && label !== project) return `${project} — ${label}`;
  return project || label || appName;
}
```
Then replace the body of the `useEffect` in the hook (currently lines 39-48) so it delegates:
```ts
  useEffect(() => {
    const title = computeWindowTitle(project, label);

    document.title = title;
    void getCurrentWindow()
      .setTitle(title)
      .catch(() => {});
  }, [project, label]);
```
Also update the example in the JSDoc comment above the hook: change `e.g. \`terax-ai — src\`` to `e.g. \`ken-ide — src\``.

- [ ] **Step 4: Run the test to verify it passes**

Run: `pnpm test -- src/modules/tabs/lib/useWindowTitle.test.ts`
Expected: PASS (5 tests).

- [ ] **Step 5: Rebrand the HTML document titles**

In `index.html` change line 7:
```html
    <title>Ken IDE</title>
```
In `settings.html` change line 7:
```html
    <title>Ken IDE — Settings</title>
```
Do NOT touch the `localStorage.getItem("terax-ui-theme-shadow")` line in either file (see Brand Values).

- [ ] **Step 6: Commit**

```bash
git add index.html settings.html src/modules/tabs/lib/useWindowTitle.ts src/modules/tabs/lib/useWindowTitle.test.ts
git commit -m "feat(rebrand): Ken IDE window and document titles"
```

---

### Task 3: Rebrand the About panel and default-theme display name

**Files:**
- Modify: `src/settings/sections/AboutSection.tsx:25,93`
- Modify: `src/modules/theme/themes/terax-default.ts:5-6`

- [ ] **Step 1: Update the About panel name fallback and Bundle ID**

In `src/settings/sections/AboutSection.tsx`, change the initial state on line 25:
```tsx
  const [name, setName] = useState("Ken IDE");
```
(`getName()` resolves to the Tauri productName at runtime; this is only the pre-resolution fallback.)

Change the displayed Bundle ID on line 93:
```tsx
        <dd className="font-mono text-[11.5px]">app.omardev.ken</dd>
```

Change the tagline on lines 77-79 from the terminal-emulator blurb:
```tsx
          <span className="text-[11px] text-muted-foreground">
            AI-native desktop IDE
          </span>
```

Leave `REPO_URL`, `WEBSITE`, and the `crynta/terax-ai` / `terax.app` link labels unchanged — they point at live infrastructure (and the updater endpoint), and must not be redirected to non-existent Ken IDE infra in this plan. Renaming them is tracked for when Ken IDE's repo/site/updater exist.

- [ ] **Step 2: Update the default-theme display strings**

In `src/modules/theme/themes/terax-default.ts`, change only the display `name` and `description` (lines 5-6), keeping `id: "terax-default"` and the `export const teraxDefault` symbol so persisted theme selections and imports stay valid:
```ts
  name: "Ken Default",
  description: "The default Ken IDE look — clean glass over neutral surfaces.",
```

- [ ] **Step 3: Verify typecheck and lint pass**

Run: `pnpm check-types`
Expected: no type errors.

Run: `pnpm lint`
Expected: no lint errors in the changed files.

- [ ] **Step 4: Commit**

```bash
git add src/settings/sections/AboutSection.tsx src/modules/theme/themes/terax-default.ts
git commit -m "feat(rebrand): Ken IDE About panel and default theme name"
```

---

### Task 4: Verify the app boots as Ken IDE and no user-facing "Terax" remains

This is the plan's done-when gate. It runs the full test/build pipeline and asserts the rebrand target files carry no leftover user-facing "Terax" strings (while allowing the deliberately-kept internal identifiers).

**Files:** none modified — verification only.

- [ ] **Step 1: Run the full frontend test suite**

Run: `pnpm test`
Expected: all tests pass, including the new `useWindowTitle.test.ts` and the existing `tabLabel.test.ts` / `eager-budget.test.ts`.

- [ ] **Step 2: Run the production frontend build**

Run: `pnpm build`
Expected: `tsc` passes and `vite build` completes with no errors.

- [ ] **Step 3: Assert no user-facing "Terax" strings remain in the rebrand targets**

Run:
```bash
grep -ni 'terax' index.html settings.html src/modules/tabs/lib/useWindowTitle.ts src/settings/sections/AboutSection.tsx src-tauri/tauri.conf.json src-tauri/tauri.windows.conf.json package.json | grep -iv 'terax-ui-theme-shadow'
```
Expected: no output (exit code 1). The only permitted match would be the `terax-ui-theme-shadow` localStorage key, which the filter removes.

Run (confirm the default-theme file kept its internal id and changed its display name):
```bash
grep -n 'terax-default\|Ken Default\|Terax Default' src/modules/theme/themes/terax-default.ts
```
Expected: `id: "terax-default"` still present, `name: "Ken Default"` present, no `Terax Default`.

- [ ] **Step 4: Manual boot smoke check**

Run: `pnpm tauri dev`
Confirm by observation:
- The window/title-bar and OS window title read `Ken IDE` on an empty launch (no project).
- Settings → About shows `Ken IDE`, Bundle ID `app.omardev.ken`, and "AI-native desktop IDE".
- The IDE chrome renders intact: left tool-window rail (Explorer / Source Control) + file tree, center editor/terminal surface, bottom status bar.
- Settings → Themes lists the default theme as `Ken Default`.

Stop the dev server after confirming.

- [ ] **Step 5: Commit (only if any verification fix was needed; otherwise skip)**

```bash
git add -A
git commit -m "chore(rebrand): verification fixes for Ken IDE rebrand"
```

---

### Task 5: Rebrand remaining user-facing strings (AI assistant, updater, settings) — folded in during execution

**Files modified:** `src/modules/ai/config.ts` (SYSTEM_PROMPT + SYSTEM_PROMPT_LITE openings → "You are Ken, an AI agent … in Ken IDE"), `src/modules/ai/components/{SelectionAskAi,AiChat,AiMiniWindow,AiComposerInput,LocalAgentNotificationsBridge}.tsx` ("Ask Ken", notification titles, `AGENT = "Ken"`), `src/modules/agents/components/NotificationBell.tsx`, `src/modules/agents/lib/agentIcon.tsx` (logo branch now matches "ken" too — fixes a regression where the renamed agent lost its logo), `src/settings/sections/{GeneralSection,ModelsSection,AgentsSection}.tsx` (→ "Ken IDE" / "Ken"), `src/modules/updater/UpdaterDialog.tsx` (dialog copy → "Ken IDE", filenames left), `src/modules/ai/lib/agent.ts` (`X-Title` → "Ken IDE").

**Verification:** `pnpm check-types` clean, `pnpm test` 158/158, and a tree-wide `grep 'Terax' src` leaves only the four intentionally-kept matches (the two deb/rpm filename templates + the two `readTeraxMd`/`TERAX.md` references).

## Known Follow-ups (out of this branch)

1. **Updater deb/rpm install-command filenames** (`UpdaterDialog.tsx:22,24`): rebrand once the real Tauri bundle artifact name is confirmed via `pnpm tauri build`. The space in `productName` "Ken IDE" likely warrants a dedicated bundle/artifact name (or quoting) so the generated `apt`/`dnf` commands are valid.
2. **External infra**: updater endpoint, GitHub repo (`crynta/terax-ai`), website (`terax.app`), and the About-panel link labels — update when Ken IDE infrastructure exists.
3. **Kept format identifiers**: `.terax-theme` extension and `TERAX.md` convention — rename only with a migration story.

## Self-Review

**Spec coverage (against `ken-ide-implementation-plans.md` Plan 2 — "Terax → Ken IDE naming; reshape layout toward §6.1 shell; done when app boots as Ken IDE with IDE chrome"):**
- Naming: Tasks 1-3 cover config, package, window/doc titles, About, default theme — every user-facing surface. ✓
- "Reshape toward §6.1 shell": finding documented — the cloned shell already realizes §6.1 (Header+search, left rail, center editor, status bar), so the slice aligns naming/branding and verifies the chrome rather than restructuring. Dockable tool-windows + Ctrl+1..N + bottom-docked terminal explicitly deferred to Plan 12. ✓
- Done-when ("boots as Ken IDE with IDE chrome"): Task 4 boot smoke + build/test gate. ✓

**Placeholder scan:** No "TBD"/"handle edge cases"/"similar to Task N". Every code step shows the exact replacement. ✓

**Type consistency:** `computeWindowTitle(project, label, appName?)` and `APP_NAME` are defined in Task 2 Step 3 and consumed identically in the Task 2 test and the hook body. Theme `id`/export symbol unchanged across Task 3. ✓
