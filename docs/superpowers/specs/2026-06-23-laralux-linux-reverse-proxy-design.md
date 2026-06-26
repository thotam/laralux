# Laralux Linux — Reverse Proxy Sites (Phase 2, Slice 3) Design Spec

**Date:** 2026-06-23
**Status:** Design approved, pending spec review
**Goal:** A site that `proxy_pass`es to one or more `host:port` upstreams instead of serving PHP, reachable at `https://<name>.dev` (with the same mkcert cert + `/etc/hosts` entry as any site). For local dev servers (Node/Vite/Next/…). Supports multiple path→upstream routes, a per-site WebSocket toggle, and editing an existing proxy.

This is **Slice 3** (final planned slice) of the Phase-2 "site management" work. Slice 1 (New Site) and Slice 2 (Site registry / Add existing folder) are merged.

---

## 1. Context & current state

After Slice 2, sites come from `core::sites::list_all_sites`, which merges `www/`-scanned sites with a registry persisted in `~/laralux/sites.toml`. The registry (`core::site_registry::SiteRegistry`) holds `RegisteredSite { name, root }` entries under `[[sites]]`. `core::sync::sync_sites` issues a mkcert cert per site, writes a per-site nginx vhost via `Site::vhost_config`, and updates the managed `/etc/hosts` block. `Site { name, root, hostname, source: SiteSource }` where `SiteSource ∈ {Scanned, Linked}`. The GUI commands `link_site`/`unlink_site` register/unregister folder sites; `unlink_site` removes a registry entry, deletes the orphaned `etc/nginx/sites/<name>.conf`, re-syncs, and reloads nginx.

This slice adds a **second registry kind** (reverse-proxy sites) that produces proxy vhosts instead of PHP/static vhosts. Folder behavior is unchanged.

## 2. Approach (chosen: separate `[[proxies]]` list + Site proxy spec)

Add a separate `proxies: Vec<ProxySite>` list to `SiteRegistry` (alongside the existing `sites`), each with a name, an ordered list of path→upstream routes, and a websocket flag. `list_all_sites` merges proxies in as `SiteSource::Proxy` carrying a `ProxySpec`; `Site::vhost_config` branches to emit a proxy server block. This keeps each struct single-purpose and is **fully backward compatible** with existing `sites.toml` files (which have only `[[sites]]`, no `[[proxies]]`).

Rejected:
- **Tagged-enum `RegisteredSite`** (`kind = "folder"|"proxy"`): old entries lack a `kind` tag and fail to deserialize; backward-compat would need awkward defaulting.
- **Optional fields on one struct** (`root: Option`, `upstream: Option`): loose, kind-dependent invariants are easy to violate.

## 3. Architecture & components

### 3.1 `core/src/site_registry.rs` — proxy entries

- `struct ProxyRoute { path: String, upstream: String }` — serde; `path` is an nginx location prefix (e.g. `/` or `/api`), `upstream` is a normalized `host:port`.
- `struct ProxySite { name: String, #[serde(default = "default_true")] websocket: bool, routes: Vec<ProxyRoute> }` — serde. `default_true` returns `true` (a proxy loaded without a `websocket` key defaults to on).
- `SiteRegistry` gains `#[serde(default)] proxies: Vec<ProxySite>`. (The existing `#[serde(default)] sites: Vec<RegisteredSite>` is unchanged — an old `sites.toml` with only `[[sites]]` still loads, `proxies` defaulting to empty.)
- New `RegistryError` variants: `InvalidUpstream(String)`, `InvalidRoute(String)`, `NoRoutes`, `NotFound(String)`.
- Pure helpers (unit-tested):
  - `normalize_upstream(input: &str) -> Result<String, RegistryError>`:
    - trims; rejects empty (`InvalidUpstream`).
    - if input has no `:`, treat it as a port → `127.0.0.1:<port>`.
    - else split once on the last `:` into `host` + `port`; host defaults to `127.0.0.1` if empty.
    - port must parse as `u16` in `1..=65535` (`InvalidUpstream` otherwise).
    - returns `"<host>:<port>"`.
  - `validate_routes(routes: &[ProxyRoute]) -> Result<Vec<ProxyRoute>, RegistryError>`:
    - `NoRoutes` if empty.
    - each `path` must start with `/` (`InvalidRoute` otherwise).
    - duplicate `path` values → `InvalidRoute`.
    - each `upstream` is run through `normalize_upstream`; returns the routes with normalized upstreams.
- Methods:
  - `add_proxy(&mut self, name: &str, routes: &[ProxyRoute], websocket: bool) -> Result<(), RegistryError>` — `validate_site_name(name)` (InvalidName); reject duplicate name across **both** `sites` and `proxies` (Duplicate); `validate_routes`; push the `ProxySite` with normalized routes.
  - `update_proxy(&mut self, name: &str, routes: &[ProxyRoute], websocket: bool) -> Result<(), RegistryError>` — find the proxy by `name` (else `NotFound`); `validate_routes`; replace its `routes` + `websocket`. (Name is the identity and is not changed here.)
  - `remove(&mut self, name: &str) -> bool` — extended to remove a matching entry from **either** `sites` or `proxies`; returns whether anything was removed.
  - `proxies(&self) -> &[ProxySite]`.

### 3.2 `core/src/sites.rs` — `SiteSource::Proxy` + proxy vhost

- `SiteSource` gains `Proxy`.
- `Site` gains `pub proxy: Option<ProxySpec>` where `ProxySpec { routes: Vec<ProxyRoute>, websocket: bool }` (re-exported from `site_registry`; serde `Serialize` so the frontend can prefill the edit modal). For `Scanned`/`Linked` sites `proxy` is `None`; `scan_sites` sets `proxy: None`.
- `list_all_sites` (after merging scanned + linked) also appends each registry proxy as `Site { name, root: PathBuf::new(), hostname: "<name>.<tld>", source: Proxy, proxy: Some(ProxySpec{routes, websocket}) }`. Dedup by name is unchanged (a scanned/linked site of the same name shadows a proxy, with a warning); final sort by name is unchanged.
- `Site::vhost_config` branches: when `self.proxy` is `Some(spec)`, emit the **proxy** server block (§4); otherwise emit the existing PHP/static block. `document_root()` is only meaningful for non-proxy sites (unchanged; not called for proxies).

### 3.3 `core/src/service/nginx.rs` — websocket upgrade map

Add to the generated `nginx.conf` **http block** (once):
```
map $http_upgrade $connection_upgrade { default upgrade; '' close; }
```
This named map is required so proxy `location`s can set `Connection $connection_upgrade` for WebSocket upgrades without breaking keep-alive on normal requests. It is harmless when no proxy site uses WebSockets. (Add a test asserting the generated config contains the map.)

### 3.4 IPC (Tauri) — `src-tauri/src/commands.rs`

- `add_proxy(app, name: String, routes: Vec<ProxyRoute>, websocket: bool) -> Result<Site, String>` — **async + spawn_blocking**, mirroring `link_site`: load registry → `add_proxy` → `save` → `sync_sites` → reload nginx if Running → return the proxy `Site` found from `list_all_sites`.
- `update_proxy(app, name: String, routes: Vec<ProxyRoute>, websocket: bool) -> Result<Site, String>` — **async + spawn_blocking**: load registry → `update_proxy` → `save` → `sync_sites` → reload nginx if Running → return the updated proxy `Site`. (Routes can change; the vhost is rewritten by `sync_sites` and nginx reloaded.)
- **Reuse `unlink_site`** unchanged (its `registry.remove(&name)` now also removes proxies, and it already deletes the orphaned vhost + re-syncs + reloads).
- `ProxyRoute` must be `Deserialize` (IPC-in) and `Serialize`; register `add_proxy`/`update_proxy` in `main.rs` `generate_handler!`.

### 3.5 Frontend — `dist/` (Reverse-proxy modal, edit, badge)

- **Sites view**: add a third action button **"Reverse proxy"** beside "New site" and "Add existing folder" (header + empty-state).
- **Proxy modal** (reuses the `ns-*` modal tokens/a11y: focus-trap, Esc, `:focus-visible`, `prefers-reduced-motion`):
  - **Site name** input — realtime validation (same rule as `validate_site_name`), live preview `→ https://<name>.dev`. In **edit mode** the name field is read-only (name is the identity; rename = Remove + recreate).
  - **WebSocket** checkbox, default checked.
  - **Routes**: a dynamic list of rows, each with a **Path** input (default `/`) and a **Target** input (`host:port`, placeholder `3000 or 127.0.0.1:5173`), plus a per-row remove button; an **"Add route"** button appends a row. At least one row is required; the first row defaults to path `/`.
  - Submit (create): `invoke("add_proxy", { name, routes, websocket })`. Submit (edit): `invoke("update_proxy", { name, routes, websocket })`. Disable form + spinner during the call; success toast (e.g. "Proxy <name> → <first upstream>"), close, refresh; error toast keeps the modal open. No `alert()`.
  - Client-side validation before submit: name valid; ≥1 route; each path starts with `/`; each target non-empty (full host:port validation is authoritative in Rust and surfaced as an error toast).
- **Proxy rows** in the sites list: show a badge **"proxy → <first route upstream>"** (append `+<N>` when there is more than one route), an **Edit** button (opens the modal in edit mode, prefilled from `site.proxy`), and a **Remove** button (`unlink_site`, two-step confirm like linked sites). **Open** opens `https://<name>.dev` via the opener plugin. Proxy rows show the upstream(s) in place of a folder path.

## 4. Proxy vhost format

For a proxy `Site` with hostname `H`, cert `C`/key `K`, and routes `[(path_i, upstream_i)]`:
```
server { listen 80; server_name H; return 301 https://$host$request_uri; }
server {
  listen 443 ssl;
  server_name H;
  ssl_certificate C; ssl_certificate_key K;
  access_log <log>/<name>-access.log;
  error_log  <log>/<name>-error.log;
  # one block per route:
  location <path_i> {
    proxy_pass http://<upstream_i>;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    # only when websocket == true:
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection $connection_upgrade;
  }
}
```
- `proxy_pass http://<upstream>;` (HTTP upstream only — https upstream is out of scope).
- nginx selects the longest matching prefix `location`, so route order in the file does not affect correctness; a `/` route acts as the catch-all.
- The `$connection_upgrade` variable is defined by the http-block map from §3.3.

## 5. Error handling

- Name/route/upstream validation is typed (`RegistryError`) → `Err(String)` → modal toast; the modal stays open for correction.
- `update_proxy` on a missing name → `NotFound` → error toast.
- A malformed `sites.toml` already degrades to "empty registry + warning" (Slice 2) — unchanged, now also covers `[[proxies]]`.
- Sync/privilege/nginx-reload failures behave as in `link_site` (best-effort; consistent with the existing commands).
- All surfaced via toasts; never `alert()`.

## 6. Testing (TDD; fakes only, no network/tools)

- `normalize_upstream`: `"3000"` → `"127.0.0.1:3000"`; `"127.0.0.1:5173"` unchanged; `"localhost:8080"` → `"localhost:8080"`; `":3000"` → `"127.0.0.1:3000"`; rejects `""`, `"0"`, `"99999"`, `"abc"`, `"127.0.0.1:abc"`.
- `validate_routes`: rejects empty (`NoRoutes`); rejects a path without leading `/`; rejects duplicate paths; normalizes each upstream.
- registry: `add_proxy` rejects a name already used by a folder site or another proxy (Duplicate) and an invalid name (InvalidName); `update_proxy` updates routes+websocket and errors `NotFound` for an unknown name; `remove` removes a proxy (returns true) and a folder (returns true) and false when absent; a `sites.toml` containing only `[[sites]]` (no `[[proxies]]`) still loads with `proxies` empty; round-trip with both lists.
- `list_all_sites`: a proxy entry appears as `SiteSource::Proxy` with `proxy: Some` carrying its routes/websocket; a proxy whose name duplicates a scanned site is shadowed + warning.
- `Site::vhost_config` (proxy): contains `proxy_pass http://127.0.0.1:3000;`, a `location /` block, and the `X-Forwarded-Proto` header; with `websocket=true` contains `proxy_set_header Upgrade $http_upgrade;` and `Connection $connection_upgrade;`, and with `websocket=false` contains neither; a two-route proxy emits two `location` blocks (`/api` and `/`). A non-proxy site still contains `fastcgi_pass`.
- nginx config: generated `nginx.conf` contains the `map $http_upgrade $connection_upgrade` line.

## 7. Out of scope (backlog)

- HTTPS-scheme upstreams (`proxy_pass https://…` + dev-cert skip-verify).
- Renaming a proxy in place (today: Remove + recreate).
- Per-route WebSocket toggles (the toggle is per-site), custom per-route headers, load-balancing across multiple upstreams for one path.
- A start/health indicator showing whether the upstream port is actually listening.
