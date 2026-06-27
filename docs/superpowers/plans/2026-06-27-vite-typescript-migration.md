# Vite + TypeScript Frontend Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single hand-written `dist/app.js` (1412-line IIFE) with a Vite + TypeScript project under `src/`, split into focused ES modules, building to `dist/` — with identical UI behavior.

**Architecture:** Vite (vanilla TS, esbuild) builds `src/` → `dist/`; Tauri loads the Vite dev server in dev and the `dist/` build in production via `beforeDevCommand`/`devUrl`/`beforeBuildCommand`. The monolithic closure is ported into modules (`state`, `ipc/*`, `ui/*`) keeping the app buildable+runnable after every task. IPC moves from `window.__TAURI__` to a typed `@tauri-apps/api` layer.

**Tech Stack:** Vite 6, TypeScript 5 (strict), `@tauri-apps/api` v2, `@tauri-apps/plugin-opener` v2, `@tauri-apps/plugin-dialog` v2. Tauri 2 (Rust unchanged).

## Global Constraints

- UI behavior, copy, and layout MUST stay identical — this is a tooling/organization migration only; no features, no UX changes.
- `dist/` becomes Vite build output (git-ignored); source lives in `src/` + repo-root `index.html`/`package.json`/`vite.config.ts`/`tsconfig.json`.
- Tauri config: `beforeDevCommand: "npm run dev"`, `devUrl: "http://localhost:1420"`, `beforeBuildCommand: "npm run build"`, `frontendDist: "../dist"` (unchanged), `withGlobalTauri: false` (only after the IPC cutover — Task 7).
- The render model stays full-`innerHTML` with exactly ONE mount site (`ui/render.ts`); morphdom is a separate later sub-project. No module other than `ui/render.ts` may assign `#app.innerHTML`.
- Rust crates are untouched: `cargo test -p laralux-core` must stay green.
- No automated JS tests exist and none are added here; each task's gate is a green type-check + build plus a manual smoke of the touched surfaces.
- Git commits have NO `Co-Authored-By` trailer. Work on the current branch (master). DO NOT create a git worktree.
- Run node/npm/cargo with `PATH="$HOME/.cargo/bin:$PATH"` (node is already on PATH at `/usr/local/bin/node`, v24.18).

## File Structure (target)

```
index.html              repo root — Vite entry (<script type=module src=/src/main.ts>)
package.json  vite.config.ts  tsconfig.json   repo root
.gitignore              + node_modules/ , /dist/
src/
  main.ts               entry: import styles, load initial state, subscribe events, bind listeners, boot render
  state.ts              the `state` object + AppState types
  ipc/
    types.ts            interfaces mirroring Rust serde structs + event payloads
    commands.ts         typed invoke wrapper per backend command
    events.ts           typed subscriptions (services-changed/sites-changed/download-progress)
  ui/
    icons.ts            the `I` SVG-string map
    util.ts             esc, validName, formatters
    toast.ts            toast list + dismiss
    render.ts           THE single innerHTML mount site
    views/  dashboard.ts setup.ts sites.ts settings.ts
    modals/ tool.ts newsite.ts linksite.ts proxy.ts domains.ts
    events.ts           delegated click/input/change/keydown dispatch → actions
  styles.css            moved verbatim from dist/styles.css; imported by main.ts
```

Extraction order keeps the app green: scaffolding → typed IPC → leaf modules → views → modals → render+events → IPC cutover → strict cleanup. Cross-module function cycles (render↔views↔actions) are fine under ES modules because every cross-call happens at call-time, not load-time.

---

### Task 1: Vite + TypeScript scaffolding (monolith ported as one module)

Stand up the build tooling and move the existing code into `src/main.ts` as ONE file, so the app builds via Vite and runs identically. Keep `window.__TAURI__` working (`withGlobalTauri` stays `true` this task), and suppress type-checking on the as-yet-untyped monolith with a single `// @ts-nocheck`.

**Files:**
- Create: `package.json`, `vite.config.ts`, `tsconfig.json` (repo root)
- Create: `index.html` (repo root)
- Create: `src/main.ts`, `src/styles.css`
- Modify: `.gitignore`, `src-tauri/tauri.conf.json`
- Delete (git rm): `dist/app.js`, `dist/index.html`, `dist/styles.css`

**Interfaces:**
- Produces: a working Vite+TS build; `src/main.ts` containing the full app (temporary `@ts-nocheck`); `src/styles.css`. Later tasks extract modules out of `src/main.ts`.

- [ ] **Step 1: Create `package.json`** (repo root)

```json
{
  "name": "laralux-frontend",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc --noEmit && vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "@tauri-apps/api": "^2",
    "@tauri-apps/plugin-dialog": "^2",
    "@tauri-apps/plugin-opener": "^2"
  },
  "devDependencies": {
    "typescript": "^5.6",
    "vite": "^6"
  }
}
```

- [ ] **Step 2: Create `vite.config.ts`** (repo root)

```ts
import { defineConfig } from "vite";

// Vite config tuned for Tauri: fixed dev port, no screen clearing, build into dist/.
export default defineConfig({
  root: ".",
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { outDir: "dist", emptyOutDir: true, target: "es2021" },
  envPrefix: ["VITE_", "TAURI_"],
});
```

- [ ] **Step 3: Create `tsconfig.json`** (repo root)

```json
{
  "compilerOptions": {
    "target": "ES2021",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "lib": ["ES2021", "DOM", "DOM.Iterable"],
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "isolatedModules": true,
    "verbatimModuleSyntax": true,
    "skipLibCheck": true,
    "noEmit": true
  },
  "include": ["src"]
}
```

- [ ] **Step 4: Create repo-root `index.html`**

```html
<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Laralux</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
```

- [ ] **Step 5: Move the stylesheet and the app code into `src/`**

```bash
git mv dist/styles.css src/styles.css
git mv dist/app.js src/main.ts
git rm dist/index.html
```
At the very top of `src/main.ts` add two lines (above the existing `/* Laralux … */` banner):
```ts
// @ts-nocheck  -- temporary: the monolith is typed module-by-module in later tasks; removed in the final task.
import "./styles.css";
```
(The existing `(() => { "use strict"; … })();` IIFE stays as-is — it still references `window.__TAURI__`, which works because `withGlobalTauri` is still true this task.)

- [ ] **Step 6: Update `.gitignore`** — append:

```
node_modules/
/dist/
```

- [ ] **Step 7: Wire Tauri to Vite** — in `src-tauri/tauri.conf.json`, replace the `build` block and flip `withGlobalTauri`:

```json
  "build": {
    "beforeDevCommand": "npm run dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "npm run build",
    "frontendDist": "../dist"
  },
```
Leave `app.withGlobalTauri` as `true` for now (it is flipped to `false` in Task 7).

- [ ] **Step 8: Install deps and verify the build**

Run:
```bash
PATH="$HOME/.cargo/bin:$PATH" npm install
PATH="$HOME/.cargo/bin:$PATH" npm run build
```
Expected: `npm install` completes; `npm run build` runs `tsc --noEmit` (passes — `@ts-nocheck` suppresses the monolith) then `vite build`, emitting `dist/index.html` + hashed `dist/assets/*.js` + `*.css`. No errors.

- [ ] **Step 9: Verify the app bundles and runs**

Run:
```bash
PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop 2>&1 | tail -3
```
Expected: `Finished`. Then smoke-test (manual): `PATH="$HOME/.cargo/bin:$PATH" cargo run -p laralux-desktop` — the app window opens and looks/behaves exactly as before (dashboard renders, Setup/Sites/Settings nav works, a tool modal opens). Close it.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "build(frontend): scaffold Vite + TypeScript, port app.js to src/main.ts"
```

---

### Task 2: Typed IPC layer (`src/ipc/*`)

Create the typed boundary to the backend. These modules are self-contained and compile under strict TS; they are wired into the app during the IPC cutover (Task 7), so this task does not yet change `main.ts` behavior.

**Files:**
- Create: `src/ipc/types.ts`, `src/ipc/commands.ts`, `src/ipc/events.ts`

**Interfaces:**
- Produces:
  - `types.ts`: `ServiceState`, `ServiceStatus`, `ToolVersion`, `ComponentStatus`, `SetupReport`, `PhpIniSettings`, `Site`, `ProxyRoute`, `ProxySpec`, `SiteDomains`, `ProgressPayload`.
  - `commands.ts`: one typed async fn per backend command (named in camelCase).
  - `events.ts`: `onServicesChanged(cb)`, `onSitesChanged(cb)`, `onDownloadProgress(cb)` returning the `UnlistenFn` promise from `@tauri-apps/api/event`.

- [ ] **Step 1: Create `src/ipc/types.ts`**

Hand-write interfaces mirroring the Rust serde structs (source of truth: `core/src/` — e.g. `service::ServiceState`, `orchestrator::ServiceStatus`, `tools::ToolVersion`, `setup::ComponentStatus`/`SetupReport`, `php_ini::PhpIniSettings`, `sites`/`site_registry`). Read those Rust types and transcribe their field names/JSON shapes. Minimum required set:

```ts
export type ServiceState = "Stopped" | "Starting" | "Running" | "Stopping" | "Crashed";

export interface ServiceStatus {
  kind: string;        // matches ServiceKind serialization used by the current UI
  state: ServiceState;
  // include every field the current app.js reads off a status object — verify against src/main.ts usage
}

export interface ToolVersion { version: string; installed: boolean; active: boolean; }

export interface ComponentStatus { component: string; present: boolean; }

export interface PhpIniSettings {
  memory_limit: string;
  upload_max_filesize: string;
  post_max_size: string;
  max_execution_time: number;
  timezone: string;
  display_errors: boolean;
  opcache_enable: boolean;
}

export interface ProgressPayload { kind: string; label?: string; done?: number; total?: number; current?: number; }

// Site / ProxyRoute / ProxySpec / SiteDomains / SetupReport: transcribe the exact
// field names the current src/main.ts reads (search it for `.root`, `.proxy`, `.routes`,
// `.upstream`, `.domains`, `rep.<field>`), so the interfaces match real usage.
```
Requirement: every field the UI actually reads must be present and correctly named. Verify field names against how `src/main.ts` consumes each value; do not invent fields.

- [ ] **Step 2: Create `src/ipc/commands.ts`** — typed wrappers over `invoke`

```ts
import { invoke } from "@tauri-apps/api/core";
import type {
  ServiceStatus, ToolVersion, ComponentStatus, SetupReport, PhpIniSettings, Site, ProxySpec,
} from "./types";

export const stackStatus = () => invoke<ServiceStatus[]>("stack_status");
export const stackStartAll = () => invoke<ServiceStatus[]>("stack_start_all");
export const stackStopAll = () => invoke<ServiceStatus[]>("stack_stop_all");
export const serviceStart = (kind: string) => invoke<ServiceStatus[]>("service_start", { kind });
export const serviceStop = (kind: string) => invoke<ServiceStatus[]>("service_stop", { kind });
export const listSites = () => invoke<Site[]>("list_sites");
export const setupStatus = () => invoke<ComponentStatus[]>("setup_status");
export const runSetupCmd = () => invoke<SetupReport>("run_setup_cmd");
export const createSite = (args: Record<string, unknown>) => invoke("create_site", args);
export const linkSite = (args: Record<string, unknown>) => invoke("link_site", args);
export const unlinkSite = (name: string) => invoke("unlink_site", { name });
export const addProxy = (args: Record<string, unknown>) => invoke("add_proxy", args);
export const updateProxy = (args: Record<string, unknown>) => invoke("update_proxy", args);
export const setSiteDomains = (args: Record<string, unknown>) => invoke("set_site_domains", args);
export const openTerminalAt = (path: string) => invoke("open_terminal", { path });
export const toolVersions = (tool: string) => invoke<ToolVersion[]>("tool_versions", { tool });
export const installToolVersion = (tool: string, version: string) => invoke<ToolVersion[]>("install_tool_version", { tool, version });
export const setToolVersion = (tool: string, version: string) => invoke<ServiceStatus[]>("set_tool_version", { tool, version });
export const toolSymlinks = () => invoke<string[]>("tool_symlinks");
export const setToolSymlink = (tool: string, enabled: boolean) => invoke<string[]>("set_tool_symlink", { tool, enabled });
export const phpIniSettings = () => invoke<PhpIniSettings>("php_ini_settings");
export const setPhpIniSettings = (settings: PhpIniSettings) => invoke<PhpIniSettings>("set_php_ini_settings", { settings });
```
Correct each arg shape against how the current `src/main.ts` calls `invoke("<cmd>", { … })` (e.g. `create_site`/`link_site`/`add_proxy` argument keys). The arg keys MUST match what the existing code passes.

- [ ] **Step 3: Create `src/ipc/events.ts`**

```ts
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ServiceStatus, ProgressPayload } from "./types";

export const onServicesChanged = (cb: (s: ServiceStatus[]) => void): Promise<UnlistenFn> =>
  listen<ServiceStatus[]>("services-changed", (e) => cb(e.payload));
export const onSitesChanged = (cb: () => void): Promise<UnlistenFn> =>
  listen("sites-changed", () => cb());
export const onDownloadProgress = (cb: (p: ProgressPayload) => void): Promise<UnlistenFn> =>
  listen<ProgressPayload>("download-progress", (e) => cb(e.payload));
```

- [ ] **Step 4: Verify type-check + build**

Run:
```bash
PATH="$HOME/.cargo/bin:$PATH" npm run build
```
Expected: green. `tsc --noEmit` type-checks the new `ipc/*` strictly (these have no `@ts-nocheck`); Vite builds. No errors. (The modules are not imported anywhere yet — that is expected; `isolatedModules`/build still passes.)

- [ ] **Step 5: Commit**

```bash
git add src/ipc
git commit -m "feat(frontend): typed IPC layer (types, command + event wrappers)"
```

---

### Task 3: Extract leaf modules (`state`, `util`, `icons`, `toast`)

Move the dependency-free pieces out of `src/main.ts` into their own modules. These have no UI dependencies, so they extract cleanly first.

**Files:**
- Create: `src/state.ts`, `src/ui/util.ts`, `src/ui/icons.ts`, `src/ui/toast.ts`
- Modify: `src/main.ts`

**Interfaces:**
- Produces: `state` (the shared mutable app-state object, exported default-style as a named `export const state`); `esc`, `validName`, and any other pure helpers from util; `I` (icon map); `toast`, `dismiss` (+ the toast array on `state`).
- Consumes: nothing new.

- [ ] **Step 1: Create `src/ui/icons.ts`** — move the `const I = { … }` icon map out of `main.ts`:
```ts
export const I = { /* …moved verbatim from main.ts… */ } as const;
```

- [ ] **Step 2: Create `src/ui/util.ts`** — move pure helpers (`esc`, `validName`, and any other standalone formatters that take args and return values without touching `state` or the DOM):
```ts
export function esc(s: string): string { /* …moved verbatim… */ }
export function validName(s: string): boolean { /* …moved verbatim… */ }
// + any other pure helpers identified in main.ts
```

- [ ] **Step 2b: Create `src/state.ts`** — move the `const state = { … }` object literal:
```ts
export const state: any = { /* …moved verbatim from main.ts… */ };
```
(Keep `: any` for now; the strict-typed `AppState` shape is introduced in the final cleanup task to avoid a giant type up front. `state` stays a single shared mutable object imported everywhere.)

- [ ] **Step 3: Create `src/ui/toast.ts`** — move the toast helpers (`toast`, `dismiss`, and the toast-rendering `toasts()` if present), importing what they need:
```ts
import { state } from "../state";
import { I } from "./icons";
export function toast(/* …same signature as in main.ts… */) { /* …moved… */ }
export function dismiss(id: number) { /* …moved… */ }
export function toasts(): string { /* …moved, if it exists… */ }
```

- [ ] **Step 4: Wire imports in `src/main.ts`**

Remove the moved declarations from `main.ts` and add at the top (after `import "./styles.css";`):
```ts
import { state } from "./state";
import { I } from "./ui/icons";
import { esc, validName } from "./ui/util";
import { toast, dismiss, toasts } from "./ui/toast";
```
`main.ts` keeps `// @ts-nocheck` for now, so its remaining references compile.

- [ ] **Step 5: Verify build + smoke**

Run:
```bash
PATH="$HOME/.cargo/bin:$PATH" npm run build && PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop 2>&1 | tail -2
```
Expected: green build + `Finished`. Smoke: `cargo run -p laralux-desktop` → trigger a toast (e.g. an action that shows one) and confirm icons render. Identical behavior.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(frontend): extract state, util, icons, toast modules"
```

---

### Task 4: Extract views (`ui/views/*`)

Move each view's render function — and the action functions that view owns — into `src/ui/views/`. Each view module exports its `…View()`/`dashboard()` render function plus its action handlers.

**Files:**
- Create: `src/ui/views/dashboard.ts`, `src/ui/views/sites.ts`, `src/ui/views/setup.ts`, `src/ui/views/settings.ts`
- Modify: `src/main.ts`

**Interfaces:**
- Produces: render fns `dashboard()`, `sitesView()`, `setupView()`, `settingsView()` (use the EXACT current names so the render() dispatch is unchanged), plus the action fns each view triggers (e.g. dashboard: `startAll`, `stopAll`, `toggleService`, `viewLogs`; sites: `createSite`/open helpers; setup: `runSetup`, `openTool`-trigger if it lives here). Keep names identical to today.
- Consumes: `state`, `esc`/`validName` (util), `I` (icons), `toast` (toast). Two temporary bridges keep extracted modules working before the later cutovers: `src/ui/legacy-invoke.ts` for `invoke` (removed in Task 7) and `src/ui/loop.ts` for `render`/`refresh` (folded into `ui/render.ts` in Task 6). These bridges exist BECAUSE `render` and `invoke` still live in `main.ts` (the entry module) this task and cannot be imported from it.

- [ ] **Step 1: Create the two transition bridges**

`src/ui/legacy-invoke.ts` (removed in Task 7):
```ts
// Temporary bridge so extracted modules keep working before the IPC cutover (Task 7).
export const invoke = (cmd: string, args?: Record<string, unknown>) =>
  (window as any).__TAURI__.core.invoke(cmd, args);
```
`src/ui/loop.ts` (folded into `ui/render.ts` in Task 6) — uses ES-module live bindings so a `render()` call made from an extracted module resolves to whatever `main.ts` registers at boot:
```ts
// Live-binding bridge for the render loop during extraction. main.ts registers the
// real functions at boot; extracted modules import { render }/{ refresh } from here.
// Folded into ui/render.ts in Task 6.
export let render: () => void = () => {};
export let refresh: () => void = () => {};
export function setLoop(r: () => void, rf: () => void) { render = r; refresh = rf; }
```

- [ ] **Step 2: Create the four view modules** — for each, move its render fn + owned actions out of `main.ts`. Example shape (`dashboard.ts`):
```ts
import { state } from "../../state";
import { esc } from "../util";
import { I } from "../icons";
import { toast } from "../toast";
import { invoke } from "../legacy-invoke";
import { render } from "../loop";

export function dashboard(): string { /* …moved verbatim… */ }
export function startAll() { /* …moved… */ }
export function stopAll() { /* …moved… */ }
export function toggleService(kind: string) { /* …moved… */ }
export function viewLogs(kind: string) { /* …moved… */ }
```
Do the same for `sites.ts` (`sitesView()` + its actions), `setup.ts` (`setupView()` + `runSetup` + setup actions), `settings.ts` (`settingsView()` + its actions). Move only what each view owns. Where a moved body calls `refresh`, also `import { refresh } from "../loop"`.

- [ ] **Step 3: Register the loop in `main.ts`** — `main.ts` still defines `render()` and `refresh()` this task. After their definitions add:
```ts
import { setLoop } from "./ui/loop";
setLoop(render, refresh);
```
so the extracted modules' `render()`/`refresh()` calls reach the real implementations. `main.ts` keeps owning `#app.innerHTML` until Task 6.

- [ ] **Step 4: Wire `main.ts`** — remove the moved fns; import them:
```ts
import { dashboard, startAll, stopAll, toggleService, viewLogs } from "./ui/views/dashboard";
import { sitesView /*, …actions */ } from "./ui/views/sites";
import { setupView /*, …actions */ } from "./ui/views/setup";
import { settingsView /*, …actions */ } from "./ui/views/settings";
```
The `render()` view-dispatch (`if state.view === "dashboard" main = dashboard() …`) now calls the imported fns.

- [ ] **Step 5: Verify build + smoke**
```bash
PATH="$HOME/.cargo/bin:$PATH" npm run build && PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop 2>&1 | tail -2
```
Expected: green + `Finished`. Smoke: open each of the 4 views; start/stop a service; nav between views. Identical.

- [ ] **Step 6: Commit**
```bash
git add -A
git commit -m "refactor(frontend): extract dashboard/sites/setup/settings views"
```

---

### Task 5: Extract modals (`ui/modals/*`)

Move each modal's render fn + its action handlers into `src/ui/modals/`.

**Files:**
- Create: `src/ui/modals/tool.ts`, `src/ui/modals/newsite.ts`, `src/ui/modals/linksite.ts`, `src/ui/modals/proxy.ts`, `src/ui/modals/domains.ts`
- Modify: `src/main.ts`

**Interfaces:**
- Produces (keep current names): `toolModal()` + `openTool`, `closeTool`, `useToolVersion`, `installToolVersion`, `toggleToolSymlink`, `applyPhpIni`; `newSiteModal()` + `openNewSite`/`closeNewSite`/`submit`; `linkSiteModal()` + its actions; `proxyModal()` + its actions; `domainsModal()` + its actions.
- Consumes: `state`, `util`, `icons`, `toast`, `render`/`refresh` (from the `ui/loop` bridge — `render.ts` does not exist until Task 6), `invoke` (the `legacy-invoke` bridge, until Task 7).

- [ ] **Step 1: Create the five modal modules** — move each modal's render fn + owned actions verbatim, with imports mirroring the view modules (Task 4 Step 2): `state`, `util`, `icons`, `toast`, `invoke` from `../legacy-invoke`, and `render`/`refresh` from `../loop`. Keep `toolModal()`'s PHP-settings section and `applyPhpIni` together in `tool.ts`.

- [ ] **Step 2: Wire `main.ts`** — remove moved fns; import them; the `render()` modal-dispatch (`state.modal === "newsite" ? newSiteModal() : …` and the `toolModal()` append) now call the imports.

- [ ] **Step 3: Verify build + smoke**
```bash
PATH="$HOME/.cargo/bin:$PATH" npm run build && PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop 2>&1 | tail -2
```
Expected: green + `Finished`. Smoke: open the PHP tool modal (versions, symlink toggle, Settings apply), new-site, link-site, proxy, domains modals. Identical, and the Task-1-era scroll fix still holds.

- [ ] **Step 4: Commit**
```bash
git add -A
git commit -m "refactor(frontend): extract tool/newsite/linksite/proxy/domains modals"
```

---

### Task 6: Extract render loop + delegated events; thin `main.ts`

Finish the structural split: `ui/render.ts` owns the single mount site; `ui/events.ts` owns the delegated dispatchers; `main.ts` becomes a thin entry.

**Files:**
- Create: `src/ui/render.ts`, `src/ui/events.ts`
- Modify: `src/main.ts`; every module that imports from `src/ui/loop.ts`
- Delete (git rm): `src/ui/loop.ts`

**Interfaces:**
- Produces: `render()` and `refresh()` in `src/ui/render.ts` — `render()` is the sole `#app.innerHTML` site (incl. the current scroll `.main`/`.modal` + focus/caret preservation); `bindEvents()` in `src/ui/events.ts` (attaches the four delegated `app`/`document` listeners and routes `data-action` to the imported action fns).
- Consumes: all view/modal render fns + action fns from Tasks 4-5; `state`; `ipc/events` (wired fully in Task 7).

- [ ] **Step 1: Move `render()` + `refresh()` into `src/ui/render.ts`** — move the bodies out of `main.ts`. `render.ts` imports the view + modal render fns and builds the HTML string, then performs the single `app.innerHTML = html` plus the existing scroll (`.main`/`.modal`) and focus/caret restoration. No other module assigns `#app.innerHTML`. Then retire the bridge: `git rm src/ui/loop.ts` and repoint every `import { render } from "../loop"` / `import { refresh } from "../loop"` (and `../../loop` in views) to `"../render"` / `"../../render"`. Remove the `setLoop(...)` call from `main.ts`.

- [ ] **Step 2: Create `src/ui/events.ts`** — move the four delegated listeners (`app.addEventListener("click"…)`, `"input"`, `"change"`, `document/app "keydown"`) into a `bindEvents()` that imports every action fn referenced by the `data-action` chains:
```ts
import { state } from "../state";
import { render } from "./render";
import { openTool, closeTool, useToolVersion, installToolVersion, toggleToolSymlink, applyPhpIni } from "./modals/tool";
import { startAll, stopAll, toggleService, viewLogs } from "./views/dashboard";
// …import the rest of the actions referenced by data-action values…
export function bindEvents() {
  const app = document.getElementById("app")!;
  app.addEventListener("click", (e) => { /* …moved dispatch chain… */ });
  app.addEventListener("input", (e) => { /* …moved… */ });
  app.addEventListener("change", (e) => { /* …moved… */ });
  document.addEventListener("keydown", (e) => { /* …moved… */ });
  app.addEventListener("keydown", (e) => { /* …moved… */ });
}
```

- [ ] **Step 3: Reduce `src/main.ts`** to the entry/boot:
```ts
// @ts-nocheck
import "./styles.css";
import { state } from "./state";
import { render } from "./ui/render";
import { bindEvents } from "./ui/events";
import { onServicesChanged, onSitesChanged, onDownloadProgress } from "./ipc/events"; // wired fully in Task 7
// keep the current boot logic: initial data load (refresh), event subscriptions, render()
// (refresh + applyServices/applyProgress move with their owners; main wires them)
bindEvents();
render();
// …boot: subscribe events + initial refresh, preserving today's behavior…
```
Keep `@ts-nocheck` on `main.ts` until Task 8.

- [ ] **Step 4: Verify build + smoke**
```bash
PATH="$HOME/.cargo/bin:$PATH" npm run build && PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop 2>&1 | tail -2
```
Expected: green + `Finished`. Smoke: full pass — every view, every modal, every `data-action` (start/stop, service toggle, logs, nav, dark toggle, new/link/proxy/domains submit, tool version use/install, symlink toggle, php apply, copy, open-url, open-terminal), and typing in inputs keeps focus. Identical.

- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "refactor(frontend): extract render loop + delegated events; thin main entry"
```

---

### Task 7: IPC cutover — typed `ipc/*`, drop `window.__TAURI__`

Replace the legacy `invoke`/`window.__TAURI__` access with the typed `ipc/` layer and the plugin packages; set `withGlobalTauri: false`.

**Files:**
- Modify: every module that imports `src/ui/legacy-invoke.ts`; `src/main.ts`; `src-tauri/tauri.conf.json`
- Delete: `src/ui/legacy-invoke.ts`

**Interfaces:**
- Consumes: `ipc/commands.ts`, `ipc/events.ts` (Task 2), `@tauri-apps/plugin-opener` (`openUrl`), `@tauri-apps/plugin-dialog` (`open`).

- [ ] **Step 1: Replace command calls** — in each view/modal module, swap `invoke("<cmd>", …)` for the matching typed fn from `../../ipc/commands` (e.g. `invoke("tool_versions", { tool })` → `toolVersions(tool)`; `invoke("set_php_ini_settings", { settings })` → `setPhpIniSettings(settings)`). Remove the `legacy-invoke` import from each.

- [ ] **Step 2: Replace events** — in `main.ts`, replace the `window.__TAURI__.event.listen(...)` boot block with `onServicesChanged`/`onSitesChanged`/`onDownloadProgress` from `ipc/events.ts`, preserving the exact current callback logic (incl. the `if (state.busy) return` guard and the `if (!state.modal) render()` calls).

- [ ] **Step 3: Replace opener + dialog** — replace `window.__TAURI__.opener.openUrl(url)` with `import { openUrl } from "@tauri-apps/plugin-opener"`; replace the `window.__TAURI__.dialog` file-picker (around the link-site browse) with `import { open } from "@tauri-apps/plugin-dialog"`. Keep the same fallback/behavior.

- [ ] **Step 4: Delete the bridge + flip the flag**
```bash
git rm src/ui/legacy-invoke.ts
```
In `src-tauri/tauri.conf.json` set `"withGlobalTauri": false`.

- [ ] **Step 5: Verify build + smoke**
```bash
PATH="$HOME/.cargo/bin:$PATH" npm run build && PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop 2>&1 | tail -2
```
Expected: green + `Finished`. Smoke (critical — IPC changed): start/stop all, service toggle, list/create/link sites, proxy add, domains edit, open terminal, open URL in browser, setup install, tool version install/use, symlink toggle, php settings apply, live `services-changed`/`sites-changed`/download-progress updates. All identical.

- [ ] **Step 6: Commit**
```bash
git add -A
git commit -m "refactor(frontend): cut over to typed @tauri-apps/api IPC; drop withGlobalTauri"
```

---

### Task 8: Strict TypeScript cleanup

Remove the temporary `@ts-nocheck`, give `state` a real `AppState` type, and resolve all remaining strict-type errors so the whole frontend type-checks.

**Files:**
- Modify: `src/main.ts` (remove `@ts-nocheck`), `src/state.ts` (typed `AppState`), and any module with residual `any`/type errors.

**Interfaces:**
- Produces: a fully strict-type-checked `src/` (no `@ts-nocheck`, no implicit `any` that `tsc --noEmit` rejects).

- [ ] **Step 1: Type `state`** — in `src/state.ts`, replace `const state: any` with an `AppState` interface covering the fields the app uses (view, modal, sites, setup, services, dark, busy, download, modal sub-shapes, etc.). Derive the shape from actual usage across the modules.

- [ ] **Step 2: Remove `@ts-nocheck` from `src/main.ts`** and fix the errors `tsc` reports (annotate event params, narrow `state.modal` unions, type the boot helpers). Prefer precise types; use `as` casts only where the DOM API genuinely requires them.

- [ ] **Step 3: Verify strict type-check + build**
```bash
PATH="$HOME/.cargo/bin:$PATH" npx tsc --noEmit
PATH="$HOME/.cargo/bin:$PATH" npm run build
```
Expected: `tsc --noEmit` reports zero errors with `strict: true` and no `@ts-nocheck` remaining; `npm run build` green.

- [ ] **Step 4: Full verification**
```bash
PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop -p laraluxctl 2>&1 | tail -2
PATH="$HOME/.cargo/bin:$PATH" cargo test -p laralux-core 2>&1 | grep "test result" | tail -1
```
Expected: `Finished`; core tests still pass (Rust untouched). Final smoke: launch the app and run the full surface pass from Task 6 Step 4 once more — everything identical to the pre-migration build.

- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "refactor(frontend): enable strict TypeScript across src/ (remove @ts-nocheck, type AppState)"
```

---

## Notes for the executor
- After every task the app MUST build (`npm run build`) and bundle (`cargo build -p laralux-desktop`) and run identically. If an extraction creates an unavoidable import cycle that breaks at load-time (not call-time), report it — the fix is to ensure the cross-module reference is used inside a function body, not at module top level.
- `dist/` is now generated; never hand-edit it. Edit `src/`.
- If `npm install` fails because `node` is missing, the user must restore the `/usr/local/bin/node` symlink (Setup → Node.js → "In terminal") or have node on PATH.
