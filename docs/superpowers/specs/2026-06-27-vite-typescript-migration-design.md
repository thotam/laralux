# Laralux — Frontend: Vite + TypeScript + Module Split

**Date:** 2026-06-27
**Status:** Design (approved for spec).
**Goal:** Replace the single hand-written `dist/app.js` (1412 lines, no build step) with a
Vite + TypeScript project whose source lives in focused ES modules under `src/`, building to
`dist/`. The UI must behave identically — this migration changes tooling and code organization
only, not behavior.

This is the first of two sequenced sub-projects. The second (a separate spec/plan) swaps the
render engine to **morphdom** so re-renders stop destroying the DOM. The module structure and the
single `render()` site defined here are chosen so morphdom slots in afterward as one dependency
import plus one function change.

---

## 1. Context & current state

- `dist/` is static and committed: `index.html` (loads `styles.css` + `app.js`), `styles.css`
  (412 lines), `app.js` (1412 lines, one IIFE/closure — everything shares scope).
- Tauri loads `dist/` directly: `tauri.conf.json` has only `build.frontendDist: "../dist"` — no
  dev server, no build step. `app.withGlobalTauri: true`, so the JS uses `window.__TAURI__`.
- Frontend ↔ backend surface: **invoke** commands (`stack_status`, `stack_start_all`,
  `stack_stop_all`, `service_start`, `service_stop`, `list_sites`, `setup_status`,
  `run_setup_cmd`, `create_site`, `link_site`, `unlink_site`, `add_proxy`, `update_proxy`,
  `set_site_domains`, `open_terminal`, `tool_versions`, `install_tool_version`,
  `set_tool_version`, `tool_symlinks`, `set_tool_symlink`, `php_ini_settings`,
  `set_php_ini_settings`); **events** the UI listens for (`download-progress`,
  `services-changed`, `sites-changed`); the **opener** plugin (`openUrl`) and the **dialog**
  plugin (file picker). The Rust side already depends on `tauri-plugin-opener` and
  `tauri-plugin-dialog`.
- `node` (v24.18, the Laralux-managed build) and `npm` (11.x) are on `PATH`.

## 2. Approach

Adopt **Vite** (the de-facto bundler for Tauri 2; native TS via esbuild, dev server with HMR,
static build output) with **TypeScript** (strict). The monolithic closure becomes ES modules under
`src/`, split by responsibility. Tauri's existing static-load is replaced by Vite's dev server in
development and Vite's `dist/` output in production, wired through Tauri's
`beforeDevCommand`/`devUrl`/`beforeBuildCommand` hooks. The IPC surface becomes a typed layer using
`@tauri-apps/api` (and the opener/dialog plugin packages) instead of `window.__TAURI__`.

The port is behavior-preserving: existing logic moves into modules verbatim (with types added); no
UX, copy, or logic changes. The render model stays full-`innerHTML` (morphdom is the next
sub-project).

## 3. Architecture & components

### 3.1 Tooling & Tauri wiring
- **`package.json`** (repo root): `type: "module"`; scripts `dev` = `vite`, `build` =
  `tsc --noEmit && vite build`, `preview` = `vite preview`. Dev deps: `vite`, `typescript`. Runtime
  deps: `@tauri-apps/api`, `@tauri-apps/plugin-opener`, `@tauri-apps/plugin-dialog`.
- **`vite.config.ts`**: `root` = repo root; `clearScreen: false`; `server: { port: 1420,
  strictPort: true }`; `build: { outDir: "dist", emptyOutDir: true, target: "es2021" }`;
  `envPrefix: ["VITE_", "TAURI_"]`.
- **`tsconfig.json`**: `strict: true`, `target: "ES2021"`, `module: "ESNext"`,
  `moduleResolution: "bundler"`, `lib: ["ES2021", "DOM", "DOM.Iterable"]`, `noEmit: true`,
  `noUnusedLocals: true`, `noUnusedParameters: true`, `isolatedModules: true`.
- **`tauri.conf.json`** `build` block becomes:
  `beforeDevCommand: "npm run dev"`, `devUrl: "http://localhost:1420"`,
  `beforeBuildCommand: "npm run build"`, `frontendDist: "../dist"` (unchanged). Set
  `app.withGlobalTauri: false`.
- **`.gitignore`** (repo root): add `node_modules/` and `/dist/`. The committed
  `dist/app.js`/`styles.css`/`index.html` are removed from the build output location (their content
  moves to `src/`); `index.html` moves to repo root as the Vite entry.
- **`index.html`** (repo root): the Vite entry — `<div id="app"></div>` and
  `<script type="module" src="/src/main.ts"></script>`; the stylesheet is imported from `main.ts`,
  not linked.

### 3.2 Source layout (`src/`)
```
index.html              (repo root — Vite entry)
package.json  vite.config.ts  tsconfig.json
src/
  main.ts               entry: import styles, load initial state, subscribe events, first render, bind delegated listeners
  state.ts              the `state` object + its types (AppState, view/modal shapes)
  ipc/
    types.ts            interfaces mirroring Rust serde structs + event payloads
    commands.ts         one typed wrapper fn per invoke command
    events.ts           subscribe to services-changed / sites-changed / download-progress
  ui/
    icons.ts            the `I` SVG-string map
    util.ts             esc, validName, formatters
    toast.ts            toast list + dismiss
    render.ts           the render() loop — THE single innerHTML mount site (morphdom replaces it later)
    views/
      dashboard.ts setup.ts sites.ts settings.ts
    modals/
      tool.ts newsite.ts linksite.ts proxy.ts domains.ts
    events.ts           delegated click/input/change/keydown dispatch → action handlers
  styles.css            (moved verbatim from dist/styles.css; imported by main.ts)
```
Each file has one responsibility. Dependency direction: `state.ts` and `ipc/types.ts` sit at the
bottom (no UI imports); `views`/`modals` import `state`, `ipc`, `util`, `icons`; `render.ts`
imports the views/modals; `main.ts` wires events and subscriptions. Action functions invoked by the
delegated dispatcher (e.g. `openTool`, `applyPhpIni`, `createSite`) are exported from their feature
module and imported by `ui/events.ts`. ES-module import cycles are avoided by keeping cross-module
calls at call-time (functions), not load-time.

### 3.3 Typed IPC layer
- `ipc/commands.ts` imports `invoke` from `@tauri-apps/api/core` and exposes one typed function per
  backend command, e.g.:
  - `stackStatus(): Promise<ServiceStatus[]>`
  - `toolVersions(tool: string): Promise<ToolVersion[]>`
  - `setToolVersion(tool: string, version: string): Promise<ServiceStatus[]>`
  - `phpIniSettings(): Promise<PhpIniSettings>` / `setPhpIniSettings(settings: PhpIniSettings): Promise<PhpIniSettings>`
  - …one for every command in §1.
- `ipc/events.ts` uses `listen` from `@tauri-apps/api/event` with typed payloads for
  `services-changed` (`ServiceStatus[]`), `sites-changed` (`void`), `download-progress`
  (`ProgressPayload`).
- `ipc/types.ts` hand-writes TS interfaces matching the Rust serde types: `ServiceState` (string
  union), `ServiceStatus`, `ToolVersion { version; installed; active }`, `ComponentStatus`,
  `PhpIniSettings` (the 7 fields), `SetupReport`, `Site`, `ProxySpec`/`ProxyRoute`, `SiteDomains`.
  These mirror the Rust structs by hand (Rust remains the source of truth; a field drift surfaces as
  a runtime mismatch, same risk as today's untyped JS — TS only adds front-end-internal safety).
- Opener: `import { openUrl } from "@tauri-apps/plugin-opener"`. Dialog: `import { open } from
  "@tauri-apps/plugin-dialog"`. These replace the `window.__TAURI__.opener`/`.dialog` access.

### 3.4 The single render site (forward-looking)
`ui/render.ts` keeps exactly one place that mounts HTML into `#app` (`app.innerHTML = html` plus the
current scroll/focus/caret preservation). Keeping this isolated is deliberate: the next sub-project
replaces that one statement with a `morphdom` call and can then delete the manual scroll/focus
restoration. No other module touches `#app.innerHTML`.

## 4. Data flow (unchanged at runtime)
1. `main.ts` runs: imports `styles.css`, loads initial data via `ipc/commands` (e.g. `stackStatus`,
   `listSites`, `setupStatus`), seeds `state`, binds the delegated listeners (`ui/events.ts`),
   subscribes to backend events (`ipc/events.ts`), and calls `render()`.
2. A user action hits a delegated listener → an action function mutates `state` and calls
   `render()` (or, for text inputs, updates `state` without a re-render, exactly as today).
3. A backend event updates `state` and calls `render()`.
4. `render()` rebuilds the HTML string from `state` and mounts it (same as today).

## 5. Behavior, build & error handling
- **Identical UI**: every view, modal, toast, progress ring, and interaction behaves and looks the
  same. No copy, layout, or logic changes.
- **Dev**: `cargo run -p laralux-desktop` triggers `beforeDevCommand` (Vite dev server on :1420) and
  loads `devUrl` with HMR. Requires `npm install` once. If `node` is unavailable (e.g. the user
  removed the `/usr/local/bin/node` symlink), the dev/build command fails fast with a clear npm
  error — documented in the README/CLAUDE notes.
- **Production**: `cargo build -p laralux-desktop` runs `beforeBuildCommand`
  (`tsc --noEmit && vite build`) → `dist/` → Tauri bundles it. A TypeScript type error fails the
  build (the `tsc --noEmit` gate), so type errors cannot ship.
- **CSP**: stays `null` (Vite dev injects module scripts; unchanged from today).

## 6. Verification
- `npm install` succeeds; `npm run build` is green (type-check + Vite build) and emits
  `dist/index.html` + hashed JS/CSS assets.
- `cargo build -p laralux-desktop` bundles without error.
- Manual smoke test (the acceptance gate — there is no automated UI test today): launch the app and
  exercise every surface — dashboard start/stop + service toggles + logs; sites list, create site
  (blank/Laravel/WordPress), link site, proxy add/edit, edit domains, open terminal, open URL, copy;
  setup install-missing + per-tool modal (version use/install, symlink toggle); PHP settings apply;
  settings view; dark-mode toggle; toasts; download progress ring. UI must match the pre-migration
  build.
- `cargo test -p laralux-core` remains green (unaffected; sanity check that the Rust side is
  untouched).

## 7. Out of scope / backlog
- **morphdom render engine** — the next sub-project (its own spec/plan); this migration only
  isolates the single render site for it.
- **CSS refactor / splitting `styles.css`** — moved verbatim, not reorganized.
- **Switching the build to emit into a different dir or adding asset hashing strategy** beyond Vite
  defaults.
- **Any new feature or UX change.**
- **Generating TS types from Rust automatically** (e.g. ts-rs/specta) — types are hand-written now;
  codegen is a possible later improvement.
