# Site Delete (confirm modal) + Single Tray Start/Stop Toggle — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user delete any site (Scanned / Linked / Proxy) through a confirmation modal — with Hide vs Delete choices for Scanned folders — and make the tray show only Start All or only Stop All depending on stack state.

**Architecture:** Core gains pure, validated filesystem ops for hiding/deleting a www folder (`core/src/sites.rs`). Two thin Tauri commands wrap them and re-sync nginx/hosts via the existing `sync_and_reload` helper. The frontend replaces the inline "Confirm remove?" toggle with a standard `.ns-overlay` modal. The tray toggles two existing menu items' visibility from the existing 1 s monitor thread.

**Tech Stack:** Rust (laralux-core, laralux-desktop/Tauri 2), TypeScript + Vite frontend (morphdom render), thiserror.

## Global Constraints

- Git commits: NO `Co-Authored-By` trailer.
- `laralux-core` keeps ZERO Tauri dependencies.
- Frontend build = `npm run build` (runs `tsc --noEmit && vite build`); the desktop loads built `dist/`, so run `npm run build` after any frontend edit.
- `tsconfig` has `strict`, `noUnusedLocals`, `noUnusedParameters` — no unused locals/params.
- Delete semantics (decided): Scanned → Hide (rename `www/<name>`→`www/.<name>`) or Delete (`remove_dir_all`); Linked/Proxy → unlink only (never touch the user's external folder).
- Tray toggle (decided): all services running → show Stop All; otherwise → show Start All.
- Name validation must reject traversal: empty, `.`, `..`, contains `/` or `\`, or leading `.`.
- Build commands run with cargo on PATH: `export PATH="$HOME/.cargo/bin:$PATH"`.

---

### Task 1: Core — validated hide/delete filesystem ops

**Files:**
- Modify: `core/src/sites.rs` (add `SiteFsError`, `valid_scanned_name`, `hide_scanned_site`, `delete_scanned_site`, and tests in the existing `#[cfg(test)] mod tests`)
- Modify: `core/src/lib.rs:49-54` area (re-export the new items)

**Interfaces:**
- Consumes: `LaraluxPaths::www()` → `PathBuf` (= `root/www`); `LaraluxPaths::new(PathBuf)`; the existing test helper `temp_root()` in `sites.rs` tests.
- Produces:
  - `pub fn valid_scanned_name(name: &str) -> bool`
  - `pub fn hide_scanned_site(paths: &LaraluxPaths, name: &str) -> Result<(), SiteFsError>`
  - `pub fn delete_scanned_site(paths: &LaraluxPaths, name: &str) -> Result<(), SiteFsError>`
  - `pub enum SiteFsError { InvalidName(String), NotFound(String), AlreadyExists(String), Io(std::io::Error) }`

- [ ] **Step 1: Write the failing tests** — append inside `core/src/sites.rs`'s `mod tests` (before its closing `}`):

```rust
    #[test]
    fn valid_scanned_name_accepts_plain_rejects_unsafe() {
        assert!(valid_scanned_name("myapp"));
        assert!(valid_scanned_name("my-app_2"));
        assert!(!valid_scanned_name(""));
        assert!(!valid_scanned_name("."));
        assert!(!valid_scanned_name(".."));
        assert!(!valid_scanned_name(".hidden"));
        assert!(!valid_scanned_name("a/b"));
        assert!(!valid_scanned_name("a\\b"));
    }

    #[test]
    fn hide_scanned_site_renames_to_dot_prefix() {
        let root = std::env::temp_dir().join(format!("lara-hide-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        std::fs::create_dir_all(paths.www().join("foo")).unwrap();

        hide_scanned_site(&paths, "foo").unwrap();
        assert!(!paths.www().join("foo").exists());
        assert!(paths.www().join(".foo").is_dir());

        // NotFound when the source dir is absent.
        assert!(matches!(hide_scanned_site(&paths, "missing"), Err(SiteFsError::NotFound(_))));
        // InvalidName for a traversing name.
        assert!(matches!(hide_scanned_site(&paths, "../x"), Err(SiteFsError::InvalidName(_))));
        // AlreadyExists when the dot-target is taken.
        std::fs::create_dir_all(paths.www().join("bar")).unwrap();
        std::fs::create_dir_all(paths.www().join(".bar")).unwrap();
        assert!(matches!(hide_scanned_site(&paths, "bar"), Err(SiteFsError::AlreadyExists(_))));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn delete_scanned_site_removes_folder() {
        let root = std::env::temp_dir().join(format!("lara-del-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        std::fs::create_dir_all(paths.www().join("foo").join("public")).unwrap();

        delete_scanned_site(&paths, "foo").unwrap();
        assert!(!paths.www().join("foo").exists());

        assert!(matches!(delete_scanned_site(&paths, "missing"), Err(SiteFsError::NotFound(_))));
        assert!(matches!(delete_scanned_site(&paths, ".."), Err(SiteFsError::InvalidName(_))));

        std::fs::remove_dir_all(&root).ok();
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p laralux-core sites:: 2>&1 | tail -20`
Expected: FAIL — `cannot find function valid_scanned_name` / `hide_scanned_site` / `delete_scanned_site` / `SiteFsError`.

- [ ] **Step 3: Implement** — add near the top of `core/src/sites.rs` (after the existing `use`/type declarations, above `scan_sites`):

```rust
#[derive(Debug, thiserror::Error)]
pub enum SiteFsError {
    #[error("invalid site name: {0}")]
    InvalidName(String),
    #[error("site folder not found: {0}")]
    NotFound(String),
    #[error("destination already exists: {0}")]
    AlreadyExists(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// True iff `name` is safe to treat as a direct child folder of `www`: non-empty,
/// not `.`/`..`, free of path separators, and not already hidden (leading `.`).
pub fn valid_scanned_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.starts_with('.')
        && !name.contains('/')
        && !name.contains('\\')
}

/// Hide a scanned site by renaming `www/<name>` → `www/.<name>`. `scan_sites`
/// skips dot-prefixed dirs, so the site vanishes from the list / hosts / nginx
/// after a re-sync while all files are kept. Reversible by renaming back.
pub fn hide_scanned_site(paths: &LaraluxPaths, name: &str) -> Result<(), SiteFsError> {
    if !valid_scanned_name(name) {
        return Err(SiteFsError::InvalidName(name.to_string()));
    }
    let src = paths.www().join(name);
    if !src.is_dir() {
        return Err(SiteFsError::NotFound(name.to_string()));
    }
    let dst = paths.www().join(format!(".{name}"));
    if dst.exists() {
        return Err(SiteFsError::AlreadyExists(format!(".{name}")));
    }
    std::fs::rename(&src, &dst)?;
    Ok(())
}

/// Permanently delete a scanned site's folder `www/<name>`.
pub fn delete_scanned_site(paths: &LaraluxPaths, name: &str) -> Result<(), SiteFsError> {
    if !valid_scanned_name(name) {
        return Err(SiteFsError::InvalidName(name.to_string()));
    }
    let dir = paths.www().join(name);
    if !dir.is_dir() {
        return Err(SiteFsError::NotFound(name.to_string()));
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}
```

- [ ] **Step 4: Re-export from `core/src/lib.rs`** — find the existing sites re-export line `pub use sites::{...}` and add the new items. The current line is:

```rust
pub use sites::{list_all_sites, scan_sites, ProxySpec, Site, SiteSource};
```

Replace with:

```rust
pub use sites::{
    delete_scanned_site, hide_scanned_site, list_all_sites, scan_sites, valid_scanned_name,
    ProxySpec, Site, SiteFsError, SiteSource,
};
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p laralux-core sites:: 2>&1 | tail -20`
Expected: PASS — the three new tests plus the existing `sites::` tests all green.

- [ ] **Step 6: Commit**

```bash
git add core/src/sites.rs core/src/lib.rs
git commit -m "feat(core): validated hide/delete ops for scanned sites"
```

---

### Task 2: Desktop — `hide_site` + `delete_site_folder` commands

**Files:**
- Modify: `src-tauri/src/commands.rs` (add two commands, after `unlink_site` which ends ~line 322)
- Modify: `src-tauri/src/main.rs:64` area (register both in `invoke_handler`)

**Interfaces:**
- Consumes: `laralux_core::hide_scanned_site`, `laralux_core::delete_scanned_site` (Task 1); existing `sync_and_reload(state: &AppState, config: &Config)` (commands.rs ~line 326); `Config::load`, `state.paths.etc_for("nginx")`, `state.paths.config_file()`, `AppState`.
- Produces: Tauri commands `hide_site(name: String)` and `delete_site_folder(name: String)`, both `-> Result<(), String>`.

- [ ] **Step 1: Implement the two commands** — add to `src-tauri/src/commands.rs` immediately after the `unlink_site` function (after its closing `}` near line 322):

```rust
/// Hide a scanned (www-folder) site: rename it to `.<name>` so it drops out of
/// the list/hosts/nginx, keeping the files. Then drop its vhost and re-sync.
#[tauri::command]
pub async fn hide_site(app: tauri::AppHandle, name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let state = app.state::<AppState>();
        laralux_core::hide_scanned_site(&state.paths, &name).map_err(|e| e.to_string())?;
        let vhost = state.paths.etc_for("nginx").join("sites").join(format!("{name}.conf"));
        let _ = std::fs::remove_file(&vhost);
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();
        sync_and_reload(&state, &config);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Permanently delete a scanned (www-folder) site's folder, drop its vhost and
/// re-sync (so /etc/hosts and nginx stop referencing it).
#[tauri::command]
pub async fn delete_site_folder(app: tauri::AppHandle, name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let state = app.state::<AppState>();
        laralux_core::delete_scanned_site(&state.paths, &name).map_err(|e| e.to_string())?;
        let vhost = state.paths.etc_for("nginx").join("sites").join(format!("{name}.conf"));
        let _ = std::fs::remove_file(&vhost);
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();
        sync_and_reload(&state, &config);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
```

- [ ] **Step 2: Register the commands** — in `src-tauri/src/main.rs`, in the `invoke_handler` list, change:

```rust
            commands::open_db_client,
        ])
```

to:

```rust
            commands::open_db_client,
            commands::hide_site,
            commands::delete_site_folder,
        ])
```

- [ ] **Step 3: Build to verify it compiles**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo build -p laralux-desktop 2>&1 | tail -5`
Expected: `Finished` with no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(desktop): hide_site + delete_site_folder commands"
```

---

### Task 3: Frontend — delete confirmation modal (state, IPC, modal, view, events)

This task is one unit: removing `confirmRemove` touches state, the sites view, and events together, so the project only compiles once all are changed.

**Files:**
- Modify: `src/state.ts` (remove `confirmRemove`; add `"deletesite"` to the `modal` union + `deleteSite` payload + default)
- Modify: `src/ipc/commands.ts` (add `hideSite`, `deleteSiteFolder`)
- Create: `src/ui/modals/deletesite.ts`
- Modify: `src/ui/render.ts:181-185` (dispatch the new modal)
- Modify: `src/ui/views/sites.ts` (kebab "Delete" for all sites; remove inline-confirm `removeSite`; add `openDeleteSite`, `closeDeleteSite`, `runDeleteAction`)
- Modify: `src/ui/events.ts` (wire `delete-site`, `ds-*`; drop `remove-site`/`confirmRemove`; Escape + focus-trap include `"deletesite"`)

**Interfaces:**
- Consumes: `hideSite(name)`, `deleteSiteFolder(name)`, existing `unlinkSite(name)`, `listSites()` from `src/ipc/commands.ts`; `state.sites: Site[]` (each has `name`, `source: "Scanned"|"Linked"|"Proxy"`, `root`, `hostname`).
- Produces (sites.ts exports): `openDeleteSite(name: string): void`, `closeDeleteSite(): void`, `runDeleteAction(fn: (name: string) => Promise<void>): Promise<void>`. The modal builder `deleteSiteModal(): string` from `src/ui/modals/deletesite.ts`.

- [ ] **Step 1: State** — in `src/state.ts`, change the `modal` union (line ~98) to include `"deletesite"`:

```ts
  modal: null | "newsite" | "linksite" | "proxy" | "domains" | "deletesite" | ToolModalState;
```

Remove the field `confirmRemove: string | null;` (line ~102) and add after `linkSite: LinkSiteState;`:

```ts
  deleteSite: null | {
    name: string;
    source: "Scanned" | "Linked" | "Proxy";
    root: string;
    url: string;
    busy: boolean;
    error: string;
  };
```

In the initial `state` object, remove `confirmRemove: null,` and add (next to `linkSite: {...},`):

```ts
  deleteSite: null,
```

- [ ] **Step 2: IPC** — in `src/ipc/commands.ts`, after the `unlinkSite` export (line ~77) add:

```ts
/** Hide a scanned site (rename its www folder to `.<name>`). */
export const hideSite = (name: string): Promise<void> =>
  invoke<void>("hide_site", { name });

/** Permanently delete a scanned site's www folder. */
export const deleteSiteFolder = (name: string): Promise<void> =>
  invoke<void>("delete_site_folder", { name });
```

- [ ] **Step 3: Modal builder** — create `src/ui/modals/deletesite.ts`:

```ts
import { state } from "../../state";
import { esc } from "../util";
import { I } from "../icons";

export function deleteSiteModal(): string {
  const d = state.deleteSite;
  if (!d) return "";
  const busy = d.busy;
  const dis = busy ? " disabled" : "";
  const err = d.error ? '<div class="ns-error">' + esc(d.error) + "</div>" : "";

  const info =
    '<div class="ns-label">' + esc(d.name) + "</div>" +
    '<div class="ds-url">' + esc(d.url) + "</div>" +
    (d.source === "Proxy" ? "" : '<div class="ds-root" title="' + esc(d.root) + '">' + esc(d.root) + "</div>");

  let body: string;
  let footer: string;
  if (d.source === "Scanned") {
    body =
      "<p>This site is a folder in <code>www</code>.</p>" +
      "<p><b>Hide</b> keeps the files and removes it from Laralux (rename the folder back to restore). " +
      "<b>Delete</b> permanently removes the folder from disk.</p>";
    footer =
      '<button class="btn btn-outline" data-action="ds-close"' + dis + ">Cancel</button>" +
      '<button class="btn" data-action="ds-hide"' + dis + ">Hide</button>" +
      '<button class="btn btn-danger" data-action="ds-delete"' + dis + ">Delete</button>";
  } else if (d.source === "Linked") {
    body =
      "<p>Removes <b>" + esc(d.name) + "</b> from Laralux. Your project folder <code>" +
      esc(d.root) + "</code> is kept.</p>";
    footer =
      '<button class="btn btn-outline" data-action="ds-close"' + dis + ">Cancel</button>" +
      '<button class="btn btn-danger" data-action="ds-remove"' + dis + ">Remove</button>";
  } else {
    body = "<p>Removes the reverse-proxy <b>" + esc(d.name) + "</b> from Laralux.</p>";
    footer =
      '<button class="btn btn-outline" data-action="ds-close"' + dis + ">Cancel</button>" +
      '<button class="btn btn-danger" data-action="ds-remove"' + dis + ">Remove</button>";
  }

  const spin = busy ? '<span class="spin spinner"></span>' : "";
  return (
    '<div class="ns-overlay" data-action="ds-overlay-click" role="dialog" aria-modal="true" aria-labelledby="ds-title">' +
    '<div class="ns-card" role="document">' +
    '<div class="ns-head"><h2 class="ns-title" id="ds-title">Delete site</h2>' +
    '<button class="icon-btn" data-action="ds-close" aria-label="Close"' + dis + ">" + I.close + "</button></div>" +
    '<div class="ns-body">' + info + body + spin + err + "</div>" +
    '<div class="ns-foot">' + footer + "</div>" +
    "</div></div>"
  );
}
```

- [ ] **Step 4: Render dispatch** — in `src/ui/render.ts`, change the modal dispatch block (lines ~181-185). First add the import near the other modal imports at the top of the file:

```ts
import { deleteSiteModal } from "./modals/deletesite";
```

Then change:

```ts
  const modalHtml = state.modal === "newsite" ? newSiteModal()
    : state.modal === "linksite" ? linkSiteModal()
    : state.modal === "proxy" ? proxyModal()
    : state.modal === "domains" ? domainsModal()
    : "";
```

to:

```ts
  const modalHtml = state.modal === "newsite" ? newSiteModal()
    : state.modal === "linksite" ? linkSiteModal()
    : state.modal === "proxy" ? proxyModal()
    : state.modal === "domains" ? domainsModal()
    : state.modal === "deletesite" ? deleteSiteModal()
    : "";
```

- [ ] **Step 5: Sites view — kebab item + handlers** — in `src/ui/views/sites.ts`:

(a) In the kebab `menuItems` string (lines ~64-70), replace the trailing Linked/Proxy-only block:

```ts
            ((isProxy || isLinked)
              ? '<button class="row-menu-item danger" data-action="remove-site" data-name="' + esc(s.name) + '">' +
                (state.confirmRemove === s.name ? "Confirm remove?" : "Remove") + "</button>"
              : "");
```

with a Delete item shown for every site:

```ts
            '<button class="row-menu-item danger" data-action="delete-site" data-name="' + esc(s.name) + '">Delete</button>';
```

(b) Replace the whole `removeSite` function (lines ~282-294) with the new handlers:

```ts
export function openDeleteSite(name: string): void {
  const s = state.sites.find((x) => x.name === name);
  if (!s) return;
  state.deleteSite = {
    name: s.name,
    source: s.source as "Scanned" | "Linked" | "Proxy",
    root: s.root,
    url: "https://" + s.hostname,
    busy: false,
    error: "",
  };
  state.rowMenu = null;
  state.modal = "deletesite";
  render();
}

export function closeDeleteSite(): void {
  if (state.deleteSite && state.deleteSite.busy) return;
  state.modal = null;
  state.deleteSite = null;
  render();
}

export async function runDeleteAction(fn: (name: string) => Promise<void>): Promise<void> {
  const d = state.deleteSite;
  if (!d || d.busy) return;
  d.busy = true;
  d.error = "";
  render();
  try {
    await fn(d.name);
    toast({ type: "success", title: "Deleted " + d.name });
    const sites = await listSites();
    state.sites = Array.isArray(sites) ? sites : [];
    state.modal = null;
    state.deleteSite = null;
    render();
  } catch (e) {
    if (state.deleteSite) {
      state.deleteSite.busy = false;
      state.deleteSite.error = String(e);
    }
    toast({ type: "error", title: "Delete failed", msg: String(e) });
    render();
  }
}
```

(c) Update the `src/ui/views/sites.ts` IPC import (lines ~6-9) — **remove `unlinkSite`** (its only use was the deleted `removeSite`; leaving it triggers `noUnusedLocals`). Do NOT add `hideSite`/`deleteSiteFolder` here — `runDeleteAction` receives the fn as a parameter; the concrete fns are imported in `events.ts`:

```ts
import {
  createSite, listSites, linkSite,
  addProxy, updateProxy, setSiteDomains, openTerminalAt, openFolderAt,
} from "../../ipc/commands";
```

- [ ] **Step 6: Events wiring** — in `src/ui/events.ts`:

(a) Update the sites import (lines ~8-14) — replace `removeSite` with the new exports:

```ts
import {
  openNewSite, closeNewSite, submitNewSite,
  openLinkSite, closeLinkSite, browseFolder, submitLinkSite,
  openProxy, closeProxy, addProxyRoute, delProxyRoute, submitProxy,
  openDomains, closeDomains, addDomainRow, delDomainRow, submitDomains,
  openDeleteSite, closeDeleteSite, runDeleteAction, copySite, openTerminal, openFolder, openExternal,
} from "./views/sites";
```

Add to the dashboard import (line ~7) nothing; add the IPC import for the delete fns at the top of events.ts (after the toast import line ~22):

```ts
import { hideSite, deleteSiteFolder, unlinkSite } from "../ipc/commands";
```

(b) In `setView` (lines ~29-33) remove `state.confirmRemove = null;` (keep `state.rowMenu = null;`).

(c) In the kebab-dismiss guard (line ~45) change the action name `remove-site` to `delete-site`:

```ts
    if (state.rowMenu && a !== "row-menu" && a !== "delete-site") {
```

(d) Replace the `remove-site` dispatch (line ~95):

```ts
    else if (a === "remove-site") removeSite(el.getAttribute("data-name")!);
```

with the new delete actions:

```ts
    else if (a === "delete-site") openDeleteSite(el.getAttribute("data-name")!);
    else if (a === "ds-close") closeDeleteSite();
    else if (a === "ds-hide") runDeleteAction(hideSite);
    else if (a === "ds-delete") runDeleteAction(deleteSiteFolder);
    else if (a === "ds-remove") runDeleteAction(unlinkSite);
    else if (a === "ds-overlay-click") { if (e.target === el) closeDeleteSite(); }
```

(e) In the Escape handler (lines ~194-199) add a branch before the `rowMenu` one:

```ts
    else if (e.key === "Escape" && state.modal === "deletesite") closeDeleteSite();
```

(f) In the focus-trap guard (line ~204) add `"deletesite"`:

```ts
    if (e.key !== "Tab" || (state.modal !== "newsite" && state.modal !== "linksite" && state.modal !== "proxy" && state.modal !== "domains" && state.modal !== "deletesite")) return;
```

- [ ] **Step 7: Styles** — append to `src/styles.css` (small helpers used by the modal):

```css
.ds-url { font-size: 13px; color: var(--text-muted); margin: 2px 0 6px; }
.ds-root { font-size: 12px; color: var(--text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; margin-bottom: 8px; }
.btn-danger { background: var(--danger, #dc2626); color: #fff; border: none; }
.btn-danger:hover { filter: brightness(0.95); }
```

- [ ] **Step 8: Build to verify (tsc strict + vite)**

Run: `npm run build 2>&1 | tail -12`
Expected: `✓ built` with no TypeScript errors (no references to `confirmRemove`/`removeSite` remain).

- [ ] **Step 9: Commit**

```bash
git add src/state.ts src/ipc/commands.ts src/ui/modals/deletesite.ts src/ui/render.ts src/ui/views/sites.ts src/ui/events.ts src/styles.css
git commit -m "feat(ui): delete-site confirmation modal for all site types"
```

---

### Task 4: Tray — single Start/Stop toggle

**Files:**
- Modify: `src-tauri/src/main.rs` (setup: initial visibility + move item clones into the monitor thread; monitor loop: toggle visibility)

**Interfaces:**
- Consumes: the `start` / `stop` `MenuItem` handles created in `setup`; `orch.snapshot()` → `Vec<laralux_core::ServiceStatus>` (each `.state == laralux_core::ServiceState::Running`); the existing monitor thread (the same one that swaps the tray icon).
- Produces: nothing new (behavioral only).

- [ ] **Step 1: Set initial visibility** — in `src-tauri/src/main.rs` setup, right after the menu is built (`let menu = MenuBuilder::new(app)...build()?;`) add:

```rust
            // Tray shows only one of Start All / Stop All; start on the stopped
            // stack so only Start All is visible. The monitor toggles them.
            let _ = start.set_visible(true);
            let _ = stop.set_visible(false);
```

- [ ] **Step 2: Move item clones into the monitor thread** — find the monitor block (`let tray = tray.clone();` then `std::thread::spawn(move || {`). Add the two clones just before `std::thread::spawn`:

```rust
                let tray = tray.clone();
                let start_item = start.clone();
                let stop_item = stop.clone();
```

- [ ] **Step 3: Toggle visibility in the loop** — inside the monitor loop, add a `last_all_running` tracker next to `last_tray`. Change:

```rust
                    let mut last: Option<Vec<laralux_core::ServiceStatus>> = None;
                    let mut last_tray: Option<TrayState> = None;
```

to:

```rust
                    let mut last: Option<Vec<laralux_core::ServiceStatus>> = None;
                    let mut last_tray: Option<TrayState> = None;
                    let mut last_all_running: Option<bool> = None;
```

Then, after the icon-update block (after `last_tray = Some(ts);` and its closing `}`), add:

```rust
                        // Tray shows only one of Start All / Stop All: all up → Stop All.
                        let all_running = !snap.is_empty()
                            && snap.iter().all(|s| s.state == laralux_core::ServiceState::Running);
                        if last_all_running != Some(all_running) {
                            let _ = start_item.set_visible(!all_running);
                            let _ = stop_item.set_visible(all_running);
                            last_all_running = Some(all_running);
                        }
```

- [ ] **Step 4: Build to verify**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo build -p laralux-desktop 2>&1 | tail -6`
Expected: `Finished` with no errors. (If `MenuItem` is not `Send` and the move fails to compile, instead keep the menu handle and resolve items each tick via `menu.get("start_all")`/`menu.get("stop_all")` — but the move is expected to compile in Tauri 2.)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "feat(tray): show only Start All or Stop All by stack state"
```

---

## Final verification (after all tasks)

- [ ] `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p laralux-core 2>&1 | grep "test result:"` — all green.
- [ ] `cargo build -p laralux-desktop 2>&1 | tail -3` — Finished.
- [ ] `npm run build 2>&1 | tail -5` — built.
- [ ] Manual smoke (`npm run build && cargo run -p laralux-desktop`): kebab → Delete on a Scanned site → modal with Hide + Delete; Hide renames folder to `.<name>` (vanishes, files intact); Delete removes the folder. Linked/Proxy → Remove unlinks (external files kept). Tray shows only Start All when stopped/partial, only Stop All when all five run.
