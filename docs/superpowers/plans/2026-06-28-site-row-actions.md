# Site-Row Actions (Open-Folder + Kebab Menu) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an "open project folder (file manager)" action to each site row and declutter the row by moving secondary actions (Copy URL, Domains, Edit, Remove) into a kebab (⋮) overflow menu, keeping terminal / open-folder / Open visible.

**Architecture:** Backend mirrors the existing open-terminal path (`core` fn → Tauri command → typed IPC wrapper → row icon button). The row keeps quick actions inline and renders a per-row `.row-menu` popover when `state.rowMenu` matches the site; the dropdown is dismissed on action / outside-click / Esc through the existing delegated handlers and the morphdom render path.

**Tech Stack:** Rust (`laralux-core`, no Tauri deps), Tauri 2 command, TypeScript (strict), Vite, morphdom.

## Global Constraints

- UI behavior of the actions themselves is unchanged — this relocates them and adds open-folder; no other UX change.
- `laralux-core` keeps ZERO Tauri dependencies.
- `render.ts` stays the SOLE `#app` mount (morphdom). No new module assigns `#app.innerHTML`.
- Strict TypeScript: no `@ts-nocheck`, no `as any` in `src/`. `npm run build` (tsc --noEmit + vite) must stay green.
- Quick cluster per row: non-proxy → `[>_]` terminal, `[📂]` open-folder, `[↗ Open]`, `[⋮]`; proxy → `[↗ Open]`, `[⋮]` only (no local folder).
- Kebab menu holds: Copy URL (`copy-site`), Domains (`edit-domains`), Edit (`edit-proxy`, proxy only), Remove (`remove-site`, proxy/linked only). Menu closes on action EXCEPT `remove-site` (keeps its `confirmRemove` two-step) and the `row-menu` toggle.
- Only one row menu open at a time (`state.rowMenu: string | null`).
- Git commits have NO `Co-Authored-By` trailer. Work on master. DO NOT create a git worktree.
- Run tools with `PATH="$HOME/.cargo/bin:$PATH"`. There are no automated UI tests; the JS/UI task's gate is a green build + manual smoke.

## File Structure

- **Create** `core/src/filemanager.rs` — detect a file manager + `open_folder(dir)`. One responsibility.
- **Modify** `core/src/lib.rs` — module + re-export.
- **Modify** `src-tauri/src/commands.rs` + `src-tauri/src/main.rs` — `open_folder` command + registration.
- **Modify** `src/ipc/commands.ts` — `openFolderAt` wrapper.
- **Modify** `src/state.ts` — `rowMenu` field.
- **Modify** `src/ui/views/sites.ts` — row markup (quick cluster + popover) + `openFolder` action fn.
- **Modify** `src/ui/events.ts` — dismiss logic, `row-menu` toggle, `open-folder` dispatch, `setView` + Esc resets.
- **Modify** `src/styles.css` — `.row-actions` / `.row-menu` styles.

---

### Task 1: Backend — `open_folder` (core + command + IPC)

Mirror the existing open-terminal path so the row's `[📂]` button can launch the user's file manager.

**Files:**
- Create: `core/src/filemanager.rs`
- Modify: `core/src/lib.rs`, `src-tauri/src/commands.rs`, `src-tauri/src/main.rs`, `src/ipc/commands.ts`

**Interfaces:**
- Produces: `laralux_core::open_folder(dir: &Path) -> Result<(), FileManagerError>`; Tauri command `open_folder(path: String) -> Result<(), String>`; `openFolderAt(path: string): Promise<void>`.

- [ ] **Step 1: Write `core/src/filemanager.rs` with the test first**

Create the file with the test module AND the implementation (the pure-logic surface is the candidate list; spawning a real file manager is not unit-tested, exactly like `terminal.rs`):

```rust
use crate::bin::resolve_bin;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum FileManagerError {
    #[error("no file manager found")]
    NoFileManager,
    #[error("failed to launch file manager: {0}")]
    Spawn(String),
}

/// Launchers that open a directory in a GUI file manager. `xdg-open` is preferred
/// (it routes to the user's default); the rest are common fallbacks. All accept
/// the directory as their sole argument.
const FILE_MANAGER_CANDIDATES: [&str; 7] = [
    "xdg-open",
    "nautilus",
    "dolphin",
    "thunar",
    "nemo",
    "pcmanfm",
    "caja",
];

/// First resolvable launcher from the candidate list, or None.
pub fn detect_file_manager() -> Option<PathBuf> {
    for c in FILE_MANAGER_CANDIDATES {
        if let Some(p) = resolve_bin(c, &[]) {
            return Some(p);
        }
    }
    None
}

/// Open `dir` in the default file manager, detached.
pub fn open_folder(dir: &Path) -> Result<(), FileManagerError> {
    let fm = detect_file_manager().ok_or(FileManagerError::NoFileManager)?;
    std::process::Command::new(&fm)
        .arg(dir)
        .spawn()
        .map_err(|e| FileManagerError::Spawn(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_open_is_preferred_and_list_is_well_formed() {
        assert_eq!(FILE_MANAGER_CANDIDATES[0], "xdg-open");
        assert_eq!(FILE_MANAGER_CANDIDATES.len(), 7);
        assert!(FILE_MANAGER_CANDIDATES.iter().all(|c| !c.is_empty()));
        assert!(FILE_MANAGER_CANDIDATES.contains(&"nautilus"));
    }
}
```

- [ ] **Step 2: Run the test (RED→GREEN in one — it's a constant invariant)**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p laralux-core filemanager 2>&1 | tail -6`
Expected: it FAILS to compile first only if `lib.rs` doesn't declare the module — so do Step 3 before running, then this passes (1 test).

- [ ] **Step 3: Wire `core/src/lib.rs`**

Add the module declaration next to `pub mod terminal;`:
```rust
pub mod filemanager;
```
And a re-export next to `pub use terminal::{open_terminal, TerminalError};`:
```rust
pub use filemanager::{open_folder, FileManagerError};
```

- [ ] **Step 4: Run the core test**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p laralux-core filemanager 2>&1 | tail -6`
Expected: PASS (`xdg_open_is_preferred_and_list_is_well_formed`), output pristine.

- [ ] **Step 5: Add the Tauri command**

In `src-tauri/src/commands.rs`, directly after the `open_terminal` command, add (mirror it exactly):
```rust
#[tauri::command]
pub fn open_folder(path: String) -> Result<(), String> {
    let dir = std::path::PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    laralux_core::open_folder(&dir).map_err(|e| e.to_string())
}
```

- [ ] **Step 6: Register the command**

In `src-tauri/src/main.rs`, add to the `tauri::generate_handler![ ... ]` list (next to `commands::open_terminal,`):
```rust
            commands::open_folder,
```

- [ ] **Step 7: Add the typed IPC wrapper**

In `src/ipc/commands.ts`, after `openTerminalAt`, add:
```ts
export const openFolderAt = (path: string): Promise<void> =>
  invoke<void>("open_folder", { path });
```

- [ ] **Step 8: Build everything**

Run:
```bash
PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop 2>&1 | tail -2
PATH="$HOME/.cargo/bin:$PATH" npm run build 2>&1 | tail -2
```
Expected: `Finished`; frontend build green (`openFolderAt` compiles even though not yet used — it's an exported const).

- [ ] **Step 9: Commit**

```bash
git add core/src/filemanager.rs core/src/lib.rs src-tauri/src/commands.rs src-tauri/src/main.rs src/ipc/commands.ts
git commit -m "feat(open-folder): core open_folder + Tauri command + typed IPC"
```

---

### Task 2: Row redesign — kebab menu + open-folder button

Restructure the site row: quick cluster (terminal, open-folder, Open, kebab) plus a per-row popover holding the secondary actions, driven by `state.rowMenu`.

**Files:**
- Modify: `src/state.ts`, `src/ui/views/sites.ts`, `src/ui/events.ts`, `src/styles.css`

**Interfaces:**
- Consumes: `openFolderAt` (Task 1); existing `toast`, `render`, `state`, `esc`, `I`.
- Produces: `state.rowMenu: string | null`; `openFolder(path)` action in `sites.ts`.

- [ ] **Step 1: Add `rowMenu` to state — `src/state.ts`**

In the `AppState` interface, after `confirmRemove: string | null;` add:
```ts
  rowMenu: string | null;
```
In the `state` object literal, after `confirmRemove: null,` add:
```ts
  rowMenu: null,
```

- [ ] **Step 2: Rewrite the row in `src/ui/views/sites.ts`**

Replace the row builder body — the block that currently computes `editBtn`/`domBtn`/`removeBtn`/`termBtn` and the `return ( '<div class="card site-row" ...> ... </div>' )` (lines ~56-77) — with this. It moves Copy/Domains/Edit/Remove into a popover and adds the folder button + kebab:

```ts
          const folderBtn = isProxy
            ? ""
            : '<button class="icon-btn sq32" data-action="open-folder" data-path="' + esc(s.root) + '" aria-label="Open folder" title="Open project folder">' + I.folder + "</button>";
          const termBtn = isProxy
            ? ""
            : '<button class="icon-btn sq32" data-action="open-terminal" data-path="' + esc(s.root) + '" aria-label="Open terminal" title="Open terminal here">' + I.terminal + "</button>";

          const menuItems =
            '<button class="row-menu-item" data-action="copy-site" data-name="' + esc(s.name) + '">' + I.copy + "Copy URL</button>" +
            '<button class="row-menu-item" data-action="edit-domains" data-name="' + esc(s.name) + '">Domains</button>' +
            (isProxy ? '<button class="row-menu-item" data-action="edit-proxy" data-name="' + esc(s.name) + '">Edit proxy</button>' : "") +
            ((isProxy || isLinked)
              ? '<button class="row-menu-item danger" data-action="remove-site" data-name="' + esc(s.name) + '">' +
                (state.confirmRemove === s.name ? "Confirm remove?" : "Remove") + "</button>"
              : "");
          const menu = state.rowMenu === s.name ? '<div class="row-menu">' + menuItems + "</div>" : "";

          const actions =
            '<div class="row-actions">' +
            termBtn + folderBtn +
            '<a class="btn-sm" href="' + esc(url) + '" data-action="open-url" data-url="' + esc(url) + '" rel="noreferrer">' + I.external + "Open</a>" +
            '<button class="icon-btn sq32" data-action="row-menu" data-name="' + esc(s.name) + '" aria-label="More actions" title="More">' + I.kebab + "</button>" +
            menu +
            "</div>";

          return (
            '<div class="card site-row" data-key="site-' + esc(s.name) + '"><div class="site-tile">' + I.folder18 + "</div>" +
            '<div class="site-info"><div class="site-name">' + esc(s.name) + "</div>" +
            '<div class="site-sub"><a class="site-url" href="' + esc(url) + '" data-action="open-url" data-url="' + esc(url) + '" rel="noreferrer">' + esc(url) + "</a>" +
            subRight + "</div></div>" +
            badge +
            actions +
            "</div>"
          );
```
(Delete the now-unused `editBtn`/`domBtn`/`removeBtn` consts — they are folded into `menuItems`. Keep `badge`, `subRight`, `target`, `url`, `isProxy`, `isLinked` exactly as they are.)

- [ ] **Step 3: Add the `openFolder` action fn + import in `src/ui/views/sites.ts`**

In the imports at the top, add `openFolderAt` to the existing `ipc/commands` import (it currently imports `openTerminalAt` and others) — e.g. `import { ..., openTerminalAt, openFolderAt } from "../../ipc/commands";` (match the existing import path/style). Then, next to the existing `openTerminal` function, add:
```ts
export async function openFolder(path: string): Promise<void> {
  try {
    await openFolderAt(path);
  } catch (e) {
    toast({ type: "error", title: "Couldn't open folder", msg: String(e) });
  }
}
```

- [ ] **Step 4: Wire events — `src/ui/events.ts`**

(a) Import the new action: add `openFolder` to the existing `import { ... } from "./views/sites";` line.

(b) Reset on view change — in `setView()`, after `state.confirmRemove = null;` add:
```ts
  state.rowMenu = null;
```

(c) Kebab dismiss — at the TOP of the click listener body (right after `const el = (e.target as Element).closest("[data-action]") as HTMLElement | null;` and computing the action), add the dismiss block. Concretely, change the top of the handler to:
```ts
  app.addEventListener("click", (e: MouseEvent) => {
    const el = (e.target as Element).closest("[data-action]") as HTMLElement | null;
    const a = el ? el.getAttribute("data-action") : null;

    // Dismiss the row kebab menu on any click except its own toggle or the
    // remove two-step. Covers outside clicks, quick buttons, and menu items.
    if (state.rowMenu && a !== "row-menu" && a !== "remove-site") {
      state.rowMenu = null;
      render();
      if (!el) return;
    }

    if (!el) return;
    // (existing chain continues; it already starts `if (a === "nav") ...`)
```
(The existing chain references `a` via `el.getAttribute("data-action")`; reuse the `a` you just computed — replace the old `const a = el.getAttribute("data-action");` line with nothing since `a` is now defined above. Ensure the chain uses the `a` variable.)

(d) Add two branches to the if/else chain — put them near the other site actions:
```ts
    else if (a === "open-folder") openFolder(el.getAttribute("data-path")!);
    else if (a === "row-menu") {
      const n = el.getAttribute("data-name")!;
      state.rowMenu = state.rowMenu === n ? null : n;
      render();
    }
```

(e) Esc dismiss — in the keydown handler, after the modal Escape branches, add:
```ts
    else if (e.key === "Escape" && state.rowMenu) { state.rowMenu = null; render(); }
```

- [ ] **Step 5: Styles — `src/styles.css`**

Append:
```css
.row-actions { position: relative; display: flex; align-items: center; gap: 6px; }
.row-menu {
  position: absolute; top: 100%; right: 0; margin-top: 6px; z-index: 30;
  min-width: 168px; padding: 4px;
  background: var(--card, #fff); border: 1px solid var(--border);
  border-radius: 10px; box-shadow: 0 14px 36px rgba(0,0,0,.3);
  display: flex; flex-direction: column;
}
.row-menu-item {
  display: flex; align-items: center; gap: 8px; width: 100%;
  padding: 8px 10px; border: 0; border-radius: 7px;
  background: transparent; color: inherit; font: inherit; font-size: 13px;
  text-align: left; cursor: pointer;
}
.row-menu-item:hover { background: var(--surface-2, rgba(127,127,127,.12)); }
.row-menu-item.danger { color: var(--danger, #dc2626); }
.row-menu-item svg { width: 15px; height: 15px; flex: none; }
```
(If any `var(--…)` name doesn't exist in this stylesheet, substitute the closest existing palette variable used by `.card`/`.btn-sm.danger`; the fallbacks after the comma keep it working regardless.)

- [ ] **Step 6: Build**

Run:
```bash
PATH="$HOME/.cargo/bin:$PATH" npm run build 2>&1 | tail -3
PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop 2>&1 | tail -2
```
Expected: green + `Finished`.

- [ ] **Step 7: Smoke test (requires a display)**

`PATH="$HOME/.cargo/bin:$PATH" cargo run -p laralux-desktop` → on Sites:
- Each non-proxy row shows `[>_] [📂] [↗ Open] [⋮]`; URL and path are no longer truncated by buttons. Proxy rows show `[↗ Open] [⋮]`.
- `[📂]` opens the project folder in the file manager.
- `[⋮]` opens the popover; Copy URL / Domains / Edit (proxy) / Remove work from it.
- The menu closes on: choosing an item (Copy/Domains/Edit), clicking outside, pressing Esc, and opening another row's menu (only one open at a time).
- Remove still uses the two-step Confirm (menu stays open between the two clicks).
Close the app. If no display, state build-only.

- [ ] **Step 8: Commit**

```bash
git add src/state.ts src/ui/views/sites.ts src/ui/events.ts src/styles.css
git commit -m "feat(ui): site-row kebab menu + open-folder button"
```

---

## Final verification
```bash
PATH="$HOME/.cargo/bin:$PATH" cargo test -p laralux-core 2>&1 | grep "test result: ok" | head -1
PATH="$HOME/.cargo/bin:$PATH" npm run build 2>&1 | tail -2
PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop -p laraluxctl 2>&1 | tail -2
grep -rn "as any\|@ts-nocheck" src/   # expect none
```
Expected: core tests pass (incl. the new filemanager test); frontend + cargo builds green; no `as any`/`@ts-nocheck`.
