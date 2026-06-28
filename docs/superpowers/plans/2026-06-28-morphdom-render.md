# morphdom Render Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single `app.innerHTML = html` mount in `src/ui/render.ts` with a morphdom diff/patch so re-renders stop destroying the DOM — preserving scroll, focus, caret, and input state natively — and key the repeated lists so mid-list edits are safe.

**Architecture:** `render()` keeps building the same `html` string from `state`, then morphs the live `#app` to match (instead of replacing it). morphdom preserves untouched nodes, so the manual `lastSig`/`lastView`/scroll/focus/caret preservation code is deleted. Repeated list items get a `data-key` and morphdom is configured to match by it.

**Tech Stack:** morphdom (runtime dep), TypeScript (strict), Vite. Builds to `dist/`; Tauri serves `dist/` via `cargo run`.

## Global Constraints

- UI behavior, copy, and layout MUST stay identical — render-engine + keys only, no feature/UX change.
- `src/ui/render.ts` stays the SOLE place that mounts into `#app` (now via morphdom, not `innerHTML`). No other module assigns `#app.innerHTML`.
- `data-key` attributes are invisible to the user and change no behavior.
- Strict TypeScript: no `@ts-nocheck`, no `as any` in `src/`. `npm run build` (tsc --noEmit + vite) must stay green.
- Rust crates are untouched.
- No automated JS tests exist and none are added; each task's gate is a green build plus a manual smoke of the touched behavior.
- Git commits have NO `Co-Authored-By` trailer. Work on the current branch (master). DO NOT create a git worktree.
- Run node/npm/cargo with `PATH="$HOME/.cargo/bin:$PATH"`.

## File Structure

- **Modify** `package.json` — add `morphdom` (and `@types/morphdom` if needed).
- **Modify** `src/ui/render.ts` — import morphdom; rewrite `render()` to morph; delete the `lastSig`/`lastView` vars and the focus/caret + scroll save-restore blocks.
- **Modify** `src/ui/views/sites.ts`, `src/ui/views/setup.ts`, `src/ui/modals/tool.ts`, `src/ui/toast.ts`, `src/ui/modals/domains.ts`, `src/ui/modals/proxy.ts` — add a `data-key` to each repeated list item.

---

### Task 1: morphdom dependency + render() swap

Replace the destroy-and-rebuild mount with a morphdom diff/patch and remove the now-redundant preservation code. After this task the app renders via morphdom; lists are still matched positionally (correct, just not yet keyed — Task 2 adds keys).

**Files:**
- Modify: `package.json` (add dependency)
- Modify: `src/ui/render.ts`

**Interfaces:**
- Consumes: existing `render()` internals (view/modal dispatch, `html` builder), `state`.
- Produces: `render()` that morphs `#app`. The `getNodeKey` it passes reads `data-key` then `id` (Task 2 supplies the `data-key`s; until then keyless items match positionally — fine).

- [ ] **Step 1: Install morphdom**

Run:
```bash
PATH="$HOME/.cargo/bin:$PATH" npm i morphdom
```
Then verify TypeScript sees types — Step 3's build will fail on the import if not. If it does, also run:
```bash
PATH="$HOME/.cargo/bin:$PATH" npm i -D @types/morphdom
```
Expected: `morphdom` appears under `dependencies` in `package.json`.

- [ ] **Step 2: Rewrite `render()` in `src/ui/render.ts`**

Add the import alongside the existing imports at the top of the file:
```ts
import morphdom from "morphdom";
```

Delete the two module-level lines (currently around line 170-171):
```ts
let lastSig = "";
let lastView: string | null = null;
```

Replace the ENTIRE `render()` function (currently lines ~173-247, from `export function render(): void {` through its closing `}` — the version that builds `html`, does `lastSig` early-return, saves focus/caret, saves `.main`/`.modal` scroll, does `app.innerHTML = html`, then restores scroll + focus) with this:

```ts
export function render(): void {
  const app = document.getElementById("app")!;
  document.documentElement.dataset.theme = state.dark ? "dark" : "light";
  let main: string;
  if (state.view === "dashboard") main = dashboard();
  else if (state.view === "sites") main = sitesView();
  else if (state.view === "setup") main = setupView();
  else main = settingsView();

  const modalHtml = state.modal === "newsite" ? newSiteModal()
    : state.modal === "linksite" ? linkSiteModal()
    : state.modal === "proxy" ? proxyModal()
    : state.modal === "domains" ? domainsModal()
    : "";
  const html =
    '<div class="root" data-compact="' + state.compact + '">' +
    header() +
    '<div class="body">' + sidebar() + '<main class="main">' + pkexecBanner() + main + "</main></div>" +
    toasts() +
    modalHtml +
    toolModal() +
    "</div>";

  // Morph the live DOM to match `html` instead of replacing it: unchanged nodes
  // survive, so scroll, focus, caret, input values, and CSS transitions are
  // preserved natively (no manual save/restore needed). Items carrying a
  // `data-key` are matched by identity so mid-list edits don't disturb siblings.
  morphdom(app, '<div id="app">' + html + '</div>', {
    childrenOnly: true,
    getNodeKey: (n) =>
      n.nodeType === 1 ? ((n as Element).getAttribute("data-key") || (n as Element).id) : "",
    onBeforeElUpdated(from) {
      // Never clobber the field the user is editing — keep its value + caret.
      if (
        from === document.activeElement &&
        (from.tagName === "INPUT" || from.tagName === "TEXTAREA")
      ) {
        return false;
      }
      return true;
    },
  });
}
```

Leave `refresh()` (below `render()`) and every other function in the file unchanged. Confirm no other code in `render.ts` still references `lastSig` or `lastView` (there should be none).

- [ ] **Step 3: Build**

Run:
```bash
PATH="$HOME/.cargo/bin:$PATH" npm run build 2>&1 | tail -4
PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop 2>&1 | tail -2
```
Expected: tsc --noEmit passes (strict, no errors), vite emits `dist/`, `cargo build` → `Finished`. If tsc errors on the morphdom import types, do the `@types/morphdom` install from Step 1 and rebuild.

- [ ] **Step 4: Smoke test (requires a display)**

Run `PATH="$HOME/.cargo/bin:$PATH" cargo run -p laralux-desktop` and verify, then close:
- Open the PHP tool modal, scroll down to Settings, toggle a setting / Apply / use a version → the modal **stays at the scrolled position** (does not jump to top).
- Type in inputs (new-site name, link-site path, proxy path/upstream, a domain, a PHP setting) while something could re-render → **focus and caret are kept**, no dropped characters, the live name-preview still updates.
- Switch views (dashboard ↔ sites ↔ setup ↔ settings) → each view starts at the top.
- Let it sit idle for a `services-changed` tick / run a service toggle → no flicker; everything updates correctly.

If you cannot run a GUI, state build-only and that the manual smoke is pending.

- [ ] **Step 5: Commit**

```bash
git add package.json package-lock.json src/ui/render.ts
git commit -m "feat(frontend): render via morphdom instead of destroying the DOM"
```

---

### Task 2: Stable `data-key` on repeated lists

Give each repeated list item a `data-key` so morphdom's `getNodeKey` (added in Task 1) matches items by identity. This makes inserting/deleting/reordering an item in the middle of a list patch only that item, leaving siblings (and their focus/scroll/transition state) untouched.

**Files:**
- Modify: `src/ui/views/sites.ts`, `src/ui/views/setup.ts`, `src/ui/modals/tool.ts`, `src/ui/toast.ts`, `src/ui/modals/domains.ts`, `src/ui/modals/proxy.ts`

**Interfaces:**
- Consumes: the `getNodeKey` from Task 1 (reads `data-key` first, then `id`).
- Produces: nothing other tasks depend on.

- [ ] **Step 1: Site rows — `src/ui/views/sites.ts`**

In the `.map((s) => {` block, the row root is (around line 68):
```ts
'<div class="card site-row"><div class="site-tile">' + I.folder18 + "</div>" +
```
Add the key to the `site-row` div:
```ts
'<div class="card site-row" data-key="site-' + esc(s.name) + '"><div class="site-tile">' + I.folder18 + "</div>" +
```

- [ ] **Step 2: Setup component rows — `src/ui/views/setup.ts`**

In the `.map((c) => {` block, the row root is (around line 23):
```ts
'<button class="setup-item setup-item-btn" data-action="open-tool" data-tool="' + esc(tk) + '">' +
```
Add the key:
```ts
'<button class="setup-item setup-item-btn" data-action="open-tool" data-tool="' + esc(tk) + '" data-key="comp-' + esc(String(c.component)) + '">' +
```

- [ ] **Step 3: Tool version rows — `src/ui/modals/tool.ts`**

In the version `.map((v) => {` block, the row is (around line 38) — note there are other `set-row` divs in this file (the symlink row, the "No versions" fallback, php-settings rows); change ONLY the one inside the version `.map`:
```ts
return '<div class="set-row"><div class="grow"><div class="t">' + esc(m.display) + " " + esc(v.version) + '</div><div class="h">' + (v.installed ? "Installed" : "Not installed") + "</div></div>" + right + "</div>";
```
Add the key to that row's opening div:
```ts
return '<div class="set-row" data-key="ver-' + esc(v.version) + '"><div class="grow"><div class="t">' + esc(m.display) + " " + esc(v.version) + '</div><div class="h">' + (v.installed ? "Installed" : "Not installed") + "</div></div>" + right + "</div>";
```

- [ ] **Step 4: Toasts — `src/ui/toast.ts`**

In the `.map((t: Toast) => {` block, the toast root is:
```ts
'<div class="toast ' +
t.type +
'" role="status"><span class="ico">' +
```
Add the key (Toast has a numeric `id`):
```ts
'<div class="toast ' +
t.type +
'" role="status" data-key="toast-' + t.id + '"><span class="ico">' +
```

- [ ] **Step 5: Domain rows — `src/ui/modals/domains.ts`**

In `sd.domains.map((v: string, i: number) =>`, the row root is (around line 11):
```ts
'<div class="pr-row">' +
```
Add the key:
```ts
'<div class="pr-row" data-key="dom-' + i + '">' +
```

- [ ] **Step 6: Proxy route rows — `src/ui/modals/proxy.ts`**

In `p.routes.map((r, i: number) =>`, the row root is (around line 13):
```ts
'<div class="pr-row">' +
```
Add the key:
```ts
'<div class="pr-row" data-key="route-' + i + '">' +
```

- [ ] **Step 7: Build**

Run:
```bash
PATH="$HOME/.cargo/bin:$PATH" npm run build 2>&1 | tail -3
PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop 2>&1 | tail -2
```
Expected: green + `Finished`.

- [ ] **Step 8: Smoke test (requires a display)**

Run `PATH="$HOME/.cargo/bin:$PATH" cargo run -p laralux-desktop` and verify, then close:
- Sites view with 3+ sites: remove a site from the **middle** of the list → the other rows stay put (no flicker/jump), final list is correct.
- New-proxy or edit-proxy modal with 3+ routes: delete a route from the **middle** → the remaining route inputs keep their values and the row you didn't touch isn't disturbed.
- Edit-domains modal with 3+ domains: delete a middle domain → remaining domain inputs are correct and undisturbed.
- Trigger several toasts, dismiss a middle one → the others remain.

If you cannot run a GUI, state build-only and that the manual smoke is pending.

- [ ] **Step 9: Commit**

```bash
git add src/ui/views/sites.ts src/ui/views/setup.ts src/ui/modals/tool.ts src/ui/toast.ts src/ui/modals/domains.ts src/ui/modals/proxy.ts
git commit -m "feat(frontend): key repeated lists for identity-based morphing"
```

---

## Final verification
```bash
PATH="$HOME/.cargo/bin:$PATH" npm run build 2>&1 | tail -3
PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop -p laraluxctl 2>&1 | tail -2
PATH="$HOME/.cargo/bin:$PATH" cargo test -p laralux-core 2>&1 | grep "test result: ok" | head -1
grep -rn "innerHTML *=" src/   # expect ONLY the preview.innerHTML lines in events.ts (ns-preview); no app.innerHTML
grep -rn "lastSig\|lastView" src/   # expect none
```
Expected: frontend + cargo build green; core tests still pass; `app.innerHTML` no longer assigned anywhere; `lastSig`/`lastView` gone.
