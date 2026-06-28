# Laralux — Site-Row Actions: Open-Folder + Kebab Overflow Menu

**Date:** 2026-06-28
**Status:** Design (approved for spec).
**Goal:** Add an "open project folder (file manager)" action to each site row, and declutter the
row — which already crowds 4-6 buttons and truncates the URL/path — by moving secondary actions
into a kebab (⋮) overflow menu, keeping only frequent actions visible.

This is a Phase-2 item ("open project folder") combined with the row redesign it requires.

---

## 1. Context & current state

- Site rows render in `src/ui/views/sites.ts`. Each row currently shows: a terminal icon
  (`open-terminal`, non-proxy only), a copy icon (`copy-site`), an `Edit` button (proxy only),
  a `Domains` button, a `Remove` button (proxy/linked only), and a primary `Open` link
  (`open-url`). With 4-6 controls the URL and filesystem path get truncated (`…`).
- The app uses a string-render model mounted via morphdom (`src/ui/render.ts`), with delegated
  click/keydown listeners bound once on `#app` (`src/ui/events.ts`). Transient row state already
  exists as `state.confirmRemove: string | null` (the Remove→Confirm pattern), reset in
  `setView()` and usable as the model to mirror.
- Icons `I.folder`/`I.folder18` and `I.kebab` already exist in `src/ui/icons.ts`.
- `open_terminal` is the backend pattern to mirror: `core/src/terminal.rs::open_terminal(dir)` →
  Tauri command `open_terminal(path)` (`src-tauri/src/commands.rs`) → `ipc/commands.ts` →
  a row icon button (`data-action="open-terminal" data-path="<root>"`).
- There is no `open_folder` yet (a core module for it was scoped earlier but never built).

## 2. Approach

Two cohesive parts:
1. **Backend open-folder**, mirroring the existing open-terminal path: a GUI-independent
   `core` function that launches the user's file manager on a directory, exposed as a Tauri command
   and a typed IPC wrapper.
2. **Row redesign**: keep the frequent actions as quick controls (terminal, the new open-folder,
   and the primary Open), and move the rest (Copy URL, Domains, Edit, Remove) into a per-row kebab
   (⋮) dropdown. The dropdown's open/closed state lives in `state.rowMenu` (one open at a time),
   rendered through the normal render path and dismissed on action / outside-click / Esc.

## 3. Architecture & components

### 3.1 `core/src/filemanager.rs` (new) — open the file manager
- `pub fn open_folder(dir: &Path) -> Result<(), FileManagerError>`: resolve a launcher via
  `crate::bin::resolve_bin` over candidates `["xdg-open", "nautilus", "dolphin", "thunar",
  "nemo", "pcmanfm", "caja"]` (first found wins), then spawn it detached with the directory as the
  sole argument (`Command::new(launcher).arg(dir).spawn()`). All of these accept `<dir>` as the
  argument, so no per-launcher arg table is needed (simpler than `terminal.rs`).
- `pub fn detect_file_manager() -> Option<PathBuf>` — the candidate resolution (unit-testable
  shape mirroring `terminal::detect_terminal`).
- `#[derive(thiserror::Error)] pub enum FileManagerError { NoFileManager, Spawn(String) }`.
- Export from `core/src/lib.rs`: `pub mod filemanager;` + `pub use filemanager::{open_folder, FileManagerError};`.

### 3.2 `src-tauri/src/commands.rs` + `main.rs` — command
- `#[tauri::command] pub fn open_folder(path: String) -> Result<(), String>`: build a `PathBuf`
  from `path` and call `laralux_core::open_folder(&dir).map_err(|e| e.to_string())` — a direct
  mirror of `open_terminal`.
- Register `commands::open_folder` in `main.rs`'s `generate_handler![]`.

### 3.3 `src/ipc/commands.ts` — typed wrapper
- `export const openFolderAt = (path: string) => invoke("open_folder", { path });`
  (mirrors `openTerminalAt`).

### 3.4 `src/state.ts` — row-menu state
- Add `rowMenu: string | null` to `AppState` (the site name whose ⋮ menu is open, or `null`),
  default `null`. Mirror `confirmRemove`'s lifecycle: also reset it to `null` in `setView()`.

### 3.5 `src/ui/views/sites.ts` — row markup
Per row the visible cluster becomes:
- Non-proxy: `[>_]` terminal (`open-terminal`), `[📂]` folder (`open-folder` `data-path="<root>"`),
  `[↗ Open]` (`open-url`), `[⋮]` (`row-menu` `data-name="<name>"`).
- Proxy: `[↗ Open]` + `[⋮]` only (no terminal/folder — no local folder), matching today's
  `isProxy` gate on the terminal button.

When `state.rowMenu === s.name`, render a `.row-menu` popover (inside the row's action cluster)
containing the secondary actions as full-width menu items, each using its EXISTING `data-action`
so no handler logic changes:
- `Copy URL` (`copy-site`), `Domains` (`edit-domains`), `Edit` (`edit-proxy`, proxy only),
  `Remove`/`Confirm?` (`remove-site`, proxy/linked only — keeps the `confirmRemove` two-step).

The kebab button carries `data-key`-friendly markup; the popover is keyed by the row's existing
`data-key="site-<name>"` parent so morphdom keeps it stable.

### 3.6 `src/ui/events.ts` — open/close behavior
- New click branch: `else if (a === "row-menu") { state.rowMenu = state.rowMenu === name ? null : name; render(); }`.
- **Dismiss on action:** the menu items reuse existing actions. `copy-site` (toast), `edit-domains`
  and `edit-proxy` (open a modal that covers the row) should close the menu — clear
  `state.rowMenu = null` when those fire (they already `render()`). `remove-site` is the ONE
  exception: it keeps its `confirmRemove` two-step, so do NOT clear `rowMenu` on a `remove-site`
  click — both clicks happen on menu items inside the open `.row-menu` (so the outside-click rule
  below won't fire), and on successful removal the row disappears entirely. Concretely: clear
  `state.rowMenu` for every dispatched action EXCEPT `row-menu` and `remove-site`.
- **Dismiss on outside click:** at the top of the click handler, if `state.rowMenu` is set and the
  clicked element is neither the `row-menu` toggle nor inside a `.row-menu`, set
  `state.rowMenu = null` (then let the normal dispatch continue and render).
- **Dismiss on Esc:** add to the keydown handler — `else if (e.key === "Escape" && state.rowMenu) { state.rowMenu = null; render(); }`.
- Only one menu open at a time (single `string | null`).

### 3.7 `src/styles.css` — popover styling
- `.row-menu`: `position:absolute`, anchored under/right of the ⋮ button (the action cluster gets
  `position:relative`); card background, border, `box-shadow`, rounded, `z-index` above rows;
  menu items reuse the existing small-button/list styling; `Remove` item keeps the `danger` accent.
- The folder quick button reuses `.icon-btn.sq32` like the terminal button.

## 4. Data flow
1. Render builds each row with the quick cluster + (when `state.rowMenu === name`) the popover.
2. Click `[⋮]` → toggle `state.rowMenu` → render shows/hides that row's popover.
3. Click a menu item → its existing action runs; `state.rowMenu` is cleared (close).
4. Click outside / Esc → `state.rowMenu = null` → render hides it.
5. Click `[📂]` → `openFolderAt(root)` → backend launches the file manager.

## 5. Behavior & error handling
- `open_folder` is best-effort like `open_terminal`: on no launcher / spawn failure it returns
  `Err`, surfaced via a toast (`"Couldn't open folder"`), matching `openTerminal`'s error toast.
- The kebab menu never blocks: outside-click and Esc always dismiss it; switching views resets it.
- Proxy rows (no local folder) show no terminal/folder buttons — unchanged from today's gate.
- morphdom: the popover is part of the row's keyed subtree, so opening/closing patches only that
  row; other rows and the open menu's own focus are undisturbed.

## 6. Testing (TDD where it applies)
- `core/src/filemanager.rs`: `detect_file_manager()` returns a candidate or `None`
  (mirror `terminal::tests`); a unit test asserting the candidate list contains `xdg-open` and
  that resolution is order-stable. (Spawning a real file manager is not unit-tested, like
  `open_terminal`.)
- `cargo test -p laralux-core` green; `cargo build -p laralux-desktop` green; `npm run build` green.
- Manual smoke (acceptance gate, needs a display):
  - A site row shows `[>_] [📂] [↗ Open] [⋮]`; the URL and path are no longer truncated by buttons.
  - `[📂]` opens the project folder in the file manager.
  - `[⋮]` opens the popover; Copy URL / Domains / Edit / Remove work from it; the menu closes on
    item click, on outside click, and on Esc; only one row's menu is open at a time.
  - Remove still uses the two-step Confirm.
  - Proxy row shows only `[↗ Open] [⋮]` (Edit/Domains/Remove/Copy in the menu).

## 7. Out of scope / backlog
- Keyboard arrow-navigation within the kebab menu (Esc-dismiss + click is enough for now).
- A DB-client button (a separate Phase-2 item).
- Any change to what the actions themselves do — this only relocates them and adds open-folder.
