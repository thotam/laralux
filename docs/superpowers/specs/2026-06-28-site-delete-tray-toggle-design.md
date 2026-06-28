# Laralux — Site delete (confirm modal) + single tray Start/Stop toggle

**Date:** 2026-06-28
**Status:** Design (approved for spec).

Two small, independent UI improvements:

1. **Delete any site via a confirmation modal** — every site (Scanned, Linked, Proxy) gets a Delete
   entry that opens a modal, replacing today's inline "Confirm remove?" text toggle that exists only
   for Linked/Proxy sites.
2. **Single tray action** — the tray shows *either* "Start All" *or* "Stop All" depending on stack
   state, not both at once.

---

## 1. Context & current state

- **Sites** have three sources (`core/src/sites.rs::SiteSource`): `Scanned` (a folder in
  `~/laralux/www/` — NOT in the registry; it is a site purely because the folder exists), `Linked`
  and `Proxy` (entries in `sites.toml` via `SiteRegistry`).
- `scan_sites` skips any www child whose name starts with `.`. `list_all_sites` = `scan_sites` +
  registry. `sync_sites` (rewrites `/etc/hosts`, regenerates nginx vhosts under
  `etc/nginx/sites/<name>.conf`) consumes `list_all_sites`. So **a www folder renamed to `.<name>`
  disappears from the list, from /etc/hosts and from nginx automatically** — no scan/sync changes
  needed.
- Today's removal (`src/ui/views/sites.ts`): the kebab menu shows a "Remove" item **only for Linked
  or Proxy** sites; clicking toggles inline text to "Confirm remove?" via `state.confirmRemove`, and a
  second click calls `unlinkSite` → the `unlink_site` command.
- `unlink_site` (`src-tauri/src/commands.rs`) is the reference flow: remove from `SiteRegistry` + save
  → delete the `sites/<name>.conf` vhost → `sync_sites` (rewrites /etc/hosts, regenerates vhosts) →
  `orch.reload(Nginx)` (SIGHUP, no rebind). It needs `PkexecPrivileged` (the hosts write may prompt).
- **Modals** follow one pattern: a builder returning `.ns-overlay > .ns-card > (.ns-head/.ns-body/
  .ns-foot)`, dispatched in `src/ui/render.ts` off `state.modal`, with Escape/overlay close and a Tab
  focus-trap in `src/ui/events.ts`. State carries a per-modal payload object (e.g. `state.newSite`).
- **Tray** (`src-tauri/src/main.rs`): a menu built once at setup with both `start_all` and `stop_all`
  items always present. A 1 s monitor thread already polls `orch.snapshot()`, recomputes `TrayState`
  and swaps the tray icon when it changes — the natural place to also toggle menu-item visibility.

## 2. Feature 1 — Delete any site via confirmation modal

### 2.1 Behavior by source
The modal's actions depend on the site's source:

- **Scanned** (laralux-managed folder in www) — two destructive choices plus Cancel:
  - **Hide** — rename `www/<name>` → `www/.<name>`. The folder and its files are kept; the site
    vanishes from Laralux, /etc/hosts and nginx (via re-sync). Reversible: rename back to restore.
  - **Delete** (danger) — `remove_dir_all(www/<name>)`: the project folder is permanently deleted.
- **Linked** — single **Remove** (danger): `unlink_site` removes the registry entry. The user's
  external project folder is **not** touched.
- **Proxy** — single **Remove** (danger): `unlink_site` removes the proxy entry (no folder exists).

After any action: re-sync (drop the vhost + /etc/hosts entry, reload nginx), refresh the site list,
toast, close the modal.

### 2.2 Core (`core/src/sites.rs`)
A name-validation guard + two filesystem operations, all scoped to www so a crafted name can never
escape it:

- `pub fn valid_scanned_name(name: &str) -> bool` — true iff non-empty, not `.`/`..`, contains no `/`
  or `\`, and does not start with `.`. (A Scanned site never starts with `.`, so this also rejects
  acting on an already-hidden folder.)
- `pub fn hide_scanned_site(paths: &LaraluxPaths, name: &str) -> Result<(), SiteFsError>` — validate;
  error if `www/<name>` is not an existing dir; error if `www/.<name>` already exists; `fs::rename`
  `www/<name>` → `www/.<name>`.
- `pub fn delete_scanned_site(paths: &LaraluxPaths, name: &str) -> Result<(), SiteFsError>` —
  validate; error if `www/<name>` is not an existing dir; `fs::remove_dir_all(www/<name>)`.
- `#[derive(thiserror::Error)] pub enum SiteFsError { InvalidName(String), NotFound(String),
  AlreadyExists(String), Io(#[from] std::io::Error) }`.
- Exported from `lib.rs`.

### 2.3 Desktop commands (`src-tauri/src/commands.rs`)
Two `#[tauri::command]`s, each in `spawn_blocking`, mirroring `unlink_site`'s tail (remove the
`sites/<name>.conf` vhost → `sync_sites` → `orch.reload(Nginx)`); factor that tail into a private
`resync_and_reload(&state)` helper reused by `unlink_site`, `hide_site` and `delete_site_folder`:

- `pub async fn hide_site(app, name: String) -> Result<(), String>`: `dbgate`-style spawn_blocking;
  `laralux_core::hide_scanned_site(&paths, &name)?`; then `resync_and_reload`.
- `pub async fn delete_site_folder(app, name: String) -> Result<(), String>`:
  `laralux_core::delete_scanned_site(&paths, &name)?`; then `resync_and_reload`.
- Register both in `main.rs` `invoke_handler`.

### 2.4 Frontend
- **State (`src/state.ts`)**: remove `confirmRemove: string | null`. Add to the `modal` union the
  literal `"deletesite"`, and a payload:
  ```ts
  deleteSite: null | { name: string; source: "Scanned" | "Linked" | "Proxy"; root: string;
                       url: string; busy: boolean; error: string };
  ```
  Default `deleteSite: null`.
- **IPC (`src/ipc/commands.ts`)**: add `hideSite(name) → invoke("hide_site", { name })` and
  `deleteSiteFolder(name) → invoke("delete_site_folder", { name })`. Keep `unlinkSite`.
- **Sites view (`src/ui/views/sites.ts`)**: in the kebab menu, replace the Linked/Proxy-only
  inline-confirm "Remove" with a `data-action="delete-site"` item shown for **every** site
  (label "Delete"). Drop the `state.confirmRemove` branch.
- **Modal (`src/ui/modals/deletesite.ts`, new)** — `deleteSiteModal()` builds the standard
  `.ns-overlay/.ns-card`. Title "Delete site". Body: site name + url + (for non-Proxy) the root path,
  and source-specific copy:
  - Scanned: "This folder lives in www. **Hide** keeps the files and removes it from Laralux (rename
    back to restore). **Delete** permanently removes the folder from disk."
  - Linked: "Removes `<name>` from Laralux. Your project folder `<root>` is kept."
  - Proxy: "Removes the reverse-proxy `<name>` from Laralux."
  - Error line (`state.deleteSite.error`) when set; buttons disabled + spinner while `busy`.
  - Footer buttons:
    - Scanned: `Cancel` (outline) · `Hide` (`data-action="ds-hide"`) · `Delete` (`data-action=
      "ds-delete"`, danger).
    - Linked/Proxy: `Cancel` · `Remove` (`data-action="ds-remove"`, danger).
- **Events (`src/ui/events.ts`)**:
  - `delete-site` → `openDeleteSite(name)`: find the site in `state.sites`, populate `state.deleteSite`
    (`busy:false, error:""`), set `state.modal = "deletesite"`, clear `rowMenu`, render.
  - `ds-hide` → `runDeleteAction(hideSite)`, `ds-delete` → `runDeleteAction(deleteSiteFolder)`,
    `ds-remove` → `runDeleteAction(unlinkSite)`. The shared `runDeleteAction(fn)` sets `busy`, awaits
    `fn(name)`, on success refreshes `listSites()` + toast + closes the modal, on error sets
    `deleteSite.error` + toast, clears `busy`, renders.
  - `ds-close` / overlay click / `Escape` (when `state.modal === "deletesite"`) → `closeDeleteSite()`
    (no-op while `busy`). Add `"deletesite"` to the Tab focus-trap and Escape lists.
  - Remove all `remove-site` / `state.confirmRemove` handling.

## 3. Feature 2 — Single tray Start/Stop toggle

- In `main.rs` setup, keep the `start` and `stop` `MenuItem` handles. Set initial visibility for the
  stopped stack: `start.set_visible(true)`, `stop.set_visible(false)`.
- Move clones of both handles into the existing 1 s monitor thread. Each tick compute
  `all_running = !snap.is_empty() && snap.iter().all(|s| s.state == ServiceState::Running)`. Track a
  `last_all_running: Option<bool>`; when it changes, `start.set_visible(!all_running)` and
  `stop.set_visible(all_running)`. (`snap` is the same `orch.snapshot()` already taken for the icon.)
- Condition (decided): **all services running → show Stop All; otherwise (none or partial) → show
  Start All** — matches the Dashboard buttons' enable logic.
- If `MenuItem<R>` is not `Send`/cannot be moved into the thread, fall back to resolving the items each
  tick from the stored menu via the app handle; otherwise prefer the moved handles.

## 4. Error handling
- Core ops return `SiteFsError`; commands map to `String` → the modal's `error` line + a toast.
  Validation rejects traversal/empty/dotted names before any fs call.
- Hide refuses when `www/.<name>` already exists (don't clobber a prior hidden copy).
- `sync_sites` failures are best-effort (logged/ignored) exactly as in `unlink_site`; the site is
  already gone from the list, and the next start re-syncs.

## 5. Testing
- `core/src/sites.rs`:
  - `valid_scanned_name`: accepts `myapp`; rejects ``, `.`, `..`, `a/b`, `a\\b`, `.hidden`.
  - `hide_scanned_site`: on a temp paths with `www/foo/`, renames to `www/.foo`; `NotFound` when the
    dir is absent; `AlreadyExists` when `www/.foo` exists; `InvalidName` for a traversing name.
  - `delete_scanned_site`: removes `www/foo` (gone afterwards); `NotFound` when absent; `InvalidName`
    for a bad name.
- `cargo test -p laralux-core` green; `cargo build -p laralux-desktop` green; `npm run build` green.
- Manual smoke: kebab → Delete on a Scanned site → modal with Hide/Delete; Hide renames to `.<name>`
  (vanishes from list, files intact); Delete removes the folder. Linked/Proxy → Remove unlinks, files
  kept. Tray shows only Start All when stopped/partial and only Stop All when all five are running.

## 6. Out of scope / backlog
- An "unhide"/show-hidden UI for `.<name>` folders (restore is manual rename for now).
- Deleting a Linked site's external folder from disk (we only unlink; the design intentionally never
  touches user folders outside www).
- Per-service tray start/stop (only the global toggle changes).
