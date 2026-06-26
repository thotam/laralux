# Laralux Linux — Site Registry / Add Existing Folder (Phase 2, Slice 2) Design Spec

**Date:** 2026-06-23
**Status:** Design approved, pending spec review
**Goal:** Let a site point at a folder **outside** `~/laralux/www/`, persisted in `~/laralux/sites.toml`, reachable at `https://<name>.dev` exactly like an auto-discovered site; and let users unlink registered sites from the UI.

This is **Slice 2** of the Phase-2 "site management" work. Slice 1 (New Site / quick app creation) is merged. Remaining slice tracked in the backlog:
- **Slice 3 — Reverse proxy sites** (a site that `proxy_pass`es to a port instead of serving PHP).

---

## 1. Context & current state

Today a "site" is purely auto-discovered: `core::sites::scan_sites` lists immediate subdirectories of `~/laralux/www/`, each becoming `Site { name, root, hostname }` with `document_root()` preferring `<root>/public`. `core::sync::sync_sites` calls `scan_sites`, issues a mkcert cert per site, writes a per-site HTTPS vhost, and updates the managed `/etc/hosts` block. The GUI `stack_start_all` and `create_site` both run `sync_sites` before serving.

Slice 2 adds a **persistent registry** of externally-located sites that merges with the `www/` scan. Because registered folders live outside `www/`, this requires a real change to how the site list is produced (a merge), unlike Slice 1.

## 2. Approach (chosen: A — Site source + merge over a TOML registry)

Add a `SiteSource` discriminator to `Site`, a new `site_registry` module that persists `{ name, root }` entries to `sites.toml`, and a `list_all_sites` merge function that combines the `www/` scan with the registry. `sync_sites` switches to the merged list, so every downstream behavior (vhosts, certs, hosts) covers linked sites for free.

Rejected:
- **B — keep two separate lists threaded through every caller.** More plumbing at every call site; the merge belongs in one place.
- **C — symlink external folders into `www/`.** Pollutes `www/`, breaks on filesystems without symlink support, confuses the scan; rejected.

## 3. Architecture & components

### 3.1 `core/src/site_registry.rs` (new)

Named `site_registry` (not `registry`) to avoid confusion with the existing `core/src/service/registry.rs` (`build_services`).

- `struct RegisteredSite { name: String, root: PathBuf }` — derives `serde::Serialize, Deserialize, Clone, PartialEq, Eq, Debug`.
- `struct SiteRegistry { #[serde(default)] sites: Vec<RegisteredSite> }` — serde; `#[serde(default)]` so an empty/missing file deserializes.
- `enum RegistryError` (thiserror): `Io`, `Parse(toml::de::Error)`, `Serialize(toml::ser::Error)`, `InvalidName(String)`, `RootNotFound(String)`, `Duplicate(String)`.
- Methods:
  - `SiteRegistry::load(path: &Path) -> Result<SiteRegistry, RegistryError>` — missing file → empty (same pattern as `Config::load`).
  - `save(&self, path: &Path) -> Result<(), RegistryError>` — create parent dir, write pretty TOML.
  - `add(&mut self, name: &str, root: &Path) -> Result<(), RegistryError>` — validates: `scaffold::validate_site_name(name)` (InvalidName on failure); `root` must exist and be a dir (RootNotFound otherwise); reject duplicate `name` already in the registry (Duplicate). Stores a canonicalized/absolute `root`.
  - `remove(&mut self, name: &str) -> bool` — removes the entry, returns whether one was removed.
- `LaraluxPaths::sites_file()` → `root.join("sites.toml")` (new accessor).

### 3.2 `core/src/sites.rs` — `SiteSource` + `list_all_sites`

- `enum SiteSource { Scanned, Linked }` — derives `Serialize, Clone, Copy, PartialEq, Eq, Debug`. Serde renders as `"Scanned"`/`"Linked"`.
- Add `pub source: SiteSource` to `struct Site`. `scan_sites` sets `SiteSource::Scanned` on every site it produces (existing tests that construct/compare `Site` are updated).
- `pub fn list_all_sites(paths: &LaraluxPaths, tld: &str) -> std::io::Result<(Vec<Site>, Vec<String>)>`:
  1. `scan_sites(paths, tld)` → scanned sites (source `Scanned`).
  2. Load the registry (`SiteRegistry::load(paths.sites_file())`); on a parse error, surface it as a warning and treat the registry as empty (don't break the whole site list over a malformed file).
  3. For each registry entry: if a scanned site already has that `name`, **skip** (scanned shadows the registry duplicate) and add a warning. If the entry's `root` no longer exists or isn't a dir, **skip** + warning ("linked site `<name>`: folder `<root>` not found"). Otherwise push `Site { name, root, hostname: "<name>.<tld>", source: Linked }`.
  4. Sort the merged list by `name`; return `(sites, warnings)`.

`document_root()` and `vhost_config()` are unchanged — linked Laravel/WordPress folders resolve `public/` automatically.

### 3.3 `core/src/sync.rs` — sync over the merged list

- `sync_sites` switches `scan_sites(paths, tld)?` → `list_all_sites(paths, tld)?` and uses the returned `Vec<Site>` for vhosts/certs/hosts exactly as today.
- Append `list_all_sites` warnings to whatever `sync_sites` returns. To carry them, `sync_sites` returns `Vec<Site>` as today **plus** the warnings — change the return type to `Result<(Vec<Site>, Vec<String>), SyncError>`. Update both callers in `commands.rs` (`stack_start_all`, `create_site`) to destructure; warnings are pushed into the relevant report's warnings / logged.

### 3.4 IPC (Tauri) — `src-tauri/src/commands.rs`

- `list_sites` → returns the merged `Vec<Site>` from `list_all_sites` (warnings ignored here; surfaced on sync). Stays synchronous (fast, no privilege/network).
- `link_site({ name: String, root: String }) -> Result<Site, String>` — **async + spawn_blocking** (it syncs + reloads nginx, like `create_site`):
  1. Load registry, `add(name, root)` (errors → `Err(String)` for toast).
  2. `save` registry.
  3. `sync_sites(...)` (vhost + cert + `/etc/hosts` via `PkexecPrivileged`).
  4. Reload nginx if Running (orchestrator `stop(Nginx)` + `start(Nginx)`) — same path `create_site` uses.
  5. Return the new `Site` (UI toasts + refreshes).
- `unlink_site({ name: String }) -> Result<(), String>` — **async + spawn_blocking**:
  1. Load registry, `remove(name)`, `save`.
  2. `sync_sites(...)` — this rewrites the `/etc/hosts` block (dropping the host) and the vhost set. Also remove the now-orphaned `etc/nginx/sites/<name>.conf` file so nginx no longer serves it.
  3. Reload nginx if Running.
- Register both in `main.rs` `generate_handler!`.

### 3.5 Native folder picker — `tauri-plugin-dialog`

Add the official dialog plugin so the modal can open a **native folder chooser** instead of relying solely on a typed path.

- `src-tauri/Cargo.toml`: add `tauri-plugin-dialog = "2"`.
- `src-tauri/src/main.rs`: `.plugin(tauri_plugin_dialog::init())` in the builder.
- `src-tauri/capabilities/default.json`: add `"dialog:allow-open"` to `permissions`.
- Frontend (with `withGlobalTauri: true`) calls `window.__TAURI__.dialog.open({ directory: true, multiple: false, title: "Choose project folder" })`. It resolves to the selected absolute path (or `null` if cancelled). No new Rust command is needed — folder selection happens entirely in the webview via the plugin; `link_site` still receives `{ name, root }`.

### 3.6 Frontend — `dist/` (modal + linked badge + remove)

- **Sites view**: add a second action button **"Add existing folder"** beside "New site" (header and empty-state).
- **Add-existing modal** (reuses the New Site modal's tokens/a11y: focus-trap, Esc, `:focus-visible`, `prefers-reduced-motion`):
  - **Folder path** row: a read-only-ish text input + a **"Browse…"** button that opens the native folder picker (§3.5). The user may also type/paste a path. Selecting a folder fills the input and auto-derives the name.
  - **Site name** input, prefilled by sanitizing the path's basename to a valid DNS label, editable; realtime validation (same rule as `validate_site_name`); live preview `→ https://<name>.dev`.
  - Submit: disable form + spinner ("Linking…"); `invoke("link_site", { name, root })`; success toast (e.g. "Linked <name> · https://<name>.dev"), close, refresh sites; error toast keeps modal open. No `alert()`.
- **Linked rows**: show a small "linked" badge and a **Remove** (unlink) button with an inline confirm (two-step click or confirm state); on confirm `invoke("unlink_site", { name })`, toast, refresh. **Scanned rows** have no Remove button (a `www/` site is removed by deleting its folder) — explain via the badge/absence.

## 4. Behavior details & decisions

- **Folder inside `www/`**: allowed to type, but if its basename collides with an auto-scanned site, the scan shadows it (`list_all_sites` skips the registry dup with a warning); effectively you can't double-register a `www/` site. Registering a *different*-named pointer to a `www/` subfolder is permitted but pointless — not specially blocked.
- **Stale linked folder** (deleted/moved after linking): kept in `sites.toml` but skipped from the live list with a warning, so it's recoverable by restoring the folder or removing the entry.
- **Hostname** equals the (validated) site name; cert + `/etc/hosts` handled by the existing `sync_sites` path. No injection surface (name is a validated DNS label).
- **Auto-DB / scaffolding**: none. Linking only registers an existing folder; it does not create files or databases (that's Slice 1's `create_site`).

## 5. Error handling

- Registry validation (bad name, missing root, duplicate) → typed `RegistryError` → `Err(String)` → modal toast; modal stays open for correction.
- Malformed `sites.toml` never breaks the site list — it degrades to "empty registry + warning".
- Sync/privilege/nginx-reload failures surface as `Err(String)` toasts (same as `create_site`).
- All surfaced via toasts; never `alert()`.

## 6. Testing (TDD; fakes only, no network/tools)

- **site_registry**: load missing file → empty; save→load roundtrip; `add` accepts a valid name + existing dir; `add` rejects invalid name (InvalidName), non-existent root (RootNotFound), duplicate (Duplicate); `remove` returns true/false correctly.
- **list_all_sites** (temp root): scanned-only path unchanged; a registry entry with an existing external dir appears as `Linked` and is sorted in; a registry entry whose name duplicates a scanned site is skipped + warning; a registry entry with a missing root is skipped + warning; `source` is set correctly on both kinds.
- **sync_sites**: writes a vhost + requests a cert for a linked site whose root is outside `www/`; returns warnings from `list_all_sites`; existing two tests updated for the new return tuple.
- **scan_sites**: existing tests updated to assert `source == Scanned` where relevant.

## 7. Out of scope (backlog)

- Reverse proxy sites (Slice 3).
- Editing an existing site's path/name (re-link); per-site PHP/Node version; open terminal/DB/folder.
