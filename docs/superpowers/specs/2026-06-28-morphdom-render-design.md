# Laralux — morphdom Render Engine

**Date:** 2026-06-28
**Status:** Design (approved for spec).
**Goal:** Stop the single `render()` from destroying and rebuilding the DOM. Replace the
`app.innerHTML = html` mount with a **morphdom** diff/patch so unchanged nodes survive a
re-render — preserving scroll, focus, caret, input state, and CSS transitions naturally, and
fixing the class of "modal jumps to top / loses focus on re-render" bugs at the source.

This is the second of the two sequenced frontend sub-projects (the first, the Vite + TypeScript
migration, is complete). It depends on that work: morphdom is added as an npm dependency and
imported into the single render site established by the migration (`src/ui/render.ts`).

---

## 1. Context & current state

- `src/ui/render.ts` exports the one `render()` that mounts the whole UI: it builds an `html`
  string from `state`, then does `app.innerHTML = html` — destroying the entire DOM subtree and
  rebuilding it every render.
- To paper over the destruction, `render()` carries manual preservation code: a `lastSig`
  early-return (skip when html is byte-identical), focus+caret save/restore (`fId`/`fAction`/
  `fIdx`/`selS`/`selE`), and scroll save/restore for `.main` (gated on `lastView`/`sameView`) and
  `.modal` (the recent commit `2a8abd8` modal-scroll fix). These exist ONLY because innerHTML
  wipes the DOM.
- Events are delegated and bound once on `#app` (`src/ui/events.ts`), so morphing children does
  not detach any listeners. No module holds a long-lived DOM node reference. These two facts make
  a DOM-morph drop-in safe.
- Text inputs update `state` on the `input` event WITHOUT a re-render (so typing does not trigger
  render); a render only fires on actions/events.

## 2. Approach

Use **morphdom** (battle-tested, MIT, dependency-free) — chosen over nanomorph/diffhtml/a
hand-rolled differ for maturity and correct handling of input `value`/`checked`/`selected`. Now
that the project has Vite + npm, morphdom is a one-line dependency and a one-function change.
TypeScript types come from the package if bundled, otherwise from `@types/morphdom` (added as a dev
dependency during implementation).

`render()` builds the same `html` string, then morphs the live `#app` to match instead of
replacing it. Because morphdom keeps unchanged nodes, all the manual preservation code becomes
unnecessary and is deleted. Repeated list items get a stable `data-key` so morphdom matches items
by identity (not position), making mid-list insert/delete/reorder safe.

## 3. Architecture & components

### 3.1 Dependency
- `npm i morphdom` (runtime dependency in `package.json`). Import as `import morphdom from "morphdom"`.
- If `tsc` reports no types for the import, also `npm i -D @types/morphdom` so the strict build stays green.

### 3.2 `src/ui/render.ts` — the morph swap
`render()` keeps building `html` exactly as today (theme dataset, view dispatch, modal dispatch,
the `.root` wrapper). It then replaces the entire `lastSig` / save / `innerHTML` / restore block
with a single morphdom call:

```ts
import morphdom from "morphdom";

export function render(): void {
  const app = document.getElementById("app")!;
  document.documentElement.dataset.theme = state.dark ? "dark" : "light";
  // ...build `main`, `modalHtml`, and `html` exactly as today...
  morphdom(app, '<div id="app">' + html + '</div>', {
    childrenOnly: true,
    getNodeKey: (n) =>
      n.nodeType === 1
        ? ((n as Element).getAttribute("data-key") || (n as Element).id)
        : "",
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

**Removed** (now redundant — morphdom preserves these natively):
- the `lastSig` field + early-return,
- the `lastView` field + `sameView` scroll gating,
- the focus+caret save/restore block,
- the `.main` + `.modal` scroll save/restore block.

`render()` shrinks to: theme + build html + morphdom call.

### 3.3 Stable keys on repeated lists
Add a `data-key` attribute to each repeated list item so morphdom matches by identity. The
`getNodeKey` above reads `data-key` first, then falls back to `id` (so existing `id`s like
`ns-name` still key correctly).

| List | File | `data-key` value |
|---|---|---|
| Site rows | `src/ui/views/sites.ts` | `site-<name>` |
| Setup component rows | `src/ui/views/setup.ts` | `comp-<component>` |
| Tool version rows | `src/ui/modals/tool.ts` | `ver-<version>` |
| Toasts | `src/ui/toast.ts` | `toast-<id>` |
| Domain rows | `src/ui/modals/domains.ts` | `dom-<idx>` |
| Proxy route rows | `src/ui/modals/proxy.ts` | `route-<idx>` |

Keys are invisible to the user (a data attribute) and change no behavior.

## 4. Data flow (unchanged)
Identical to today: an action/event mutates `state` and calls `render()`; `render()` rebuilds the
`html` string from `state`. The only change is the final step — morph instead of replace. morphdom
walks the new tree against the live DOM, updates only what differs (keyed by `data-key`/`id`), and
skips the focused input via `onBeforeElUpdated`.

## 5. Behavior & error handling
- **Identical UI, better preservation:** every view/modal/toast/interaction looks and behaves the
  same; additionally, scroll position, focus, caret, in-progress input values, and CSS transitions
  survive re-renders because nodes are not destroyed.
- **View switch:** changing `state.view` produces a different `.main` subtree; morphdom replaces
  that subtree, so the new view naturally starts at the top (matches today's "navigation resets
  scroll").
- **Always-morph (no `lastSig`):** morphdom already minimizes DOM mutations (no-op when html is
  unchanged) and self-heals any drift from the input listener's direct DOM tweaks. `render()` is
  low-frequency (event-driven; download-progress updates the ring directly, not via render), so the
  removed string pre-check costs nothing measurable.
- **Mid-list edit safety:** with `data-key`, deleting/inserting an item in the middle of a list
  patches only that item; the other rows keep their DOM nodes (and any focus/scroll/transition).

## 6. Verification (no automated UI tests exist; gate = build + manual smoke)
- `npm run build` (tsc strict + vite) green; `cargo build -p laralux-desktop` Finished.
- Manual smoke (the acceptance gate, requires a display):
  - Scroll down in the PHP tool modal, then toggle a setting / Apply / use a version → **modal stays put** (no jump to top).
  - Type in inputs (new-site name, link-site path, proxy path/upstream, domain, PHP settings) → **focus and caret are kept**, no character loss, live preview still updates.
  - Switch views (dashboard ↔ sites ↔ setup ↔ settings) → new view starts at top; returning preserves nothing stale.
  - Delete a site / a domain row / a proxy route **from the middle** of its list → remaining rows are undisturbed; final list is correct.
  - Live `services-changed` update while idle → no flicker; dark-mode toggle, toasts, setup install progress all behave as before.
  - Rapid repeated renders (toggle back and forth) → no flicker, no leaked DOM.

## 7. Out of scope / backlog
- Any UI/behavior/feature change (this is render-engine + keys only).
- Keyed morphing for lists not listed in §3.3 (none others are dynamic enough to need it).
- Replacing the delegated-events or state model (unchanged).
- Server-side/SSR or virtual-DOM frameworks (morphdom keeps the string-render model intact).
