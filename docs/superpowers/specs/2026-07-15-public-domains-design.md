# Public domains (real domains via upstream reverse-proxy) — Design

**Date:** 2026-07-15
**Status:** Approved (pending spec review)

## Problem

Sites in laralux are served at `<name>.<tld>` (default `.dev`) over HTTPS using a
locally-trusted mkcert certificate, with the hostname pinned to `127.0.0.1` via a
managed `/etc/hosts` block. This only works on the local machine.

The user runs a public server that owns a real domain (e.g. `app.example.com`),
terminates TLS there (Let's Encrypt), and reverse-proxies plain HTTP down to the
device running laralux. The device therefore needs to **serve that real domain
over HTTP** — without the local-only machinery getting in the way.

The backend already accepts arbitrary DNS domains (`validate_domain`, the "Edit
domains" modal). The gap is not "allow real domains" — it is the local-only
behaviour that breaks the upstream-proxy flow:

1. `sync_sites` pins **every** domain (including real ones) to `127.0.0.1` in
   `/etc/hosts` — wrong for a domain owned by a separate public server.
2. Every vhost forces `listen 80; return 301 https://…`, so an upstream that
   proxies over plain HTTP hits a redirect loop.
3. mkcert issues a locally-trusted cert that public browsers reject (irrelevant
   here since TLS terminates upstream, but wasted work / noise).

## Chosen approach

**Server terminates TLS, proxies HTTP** to the device (confirmed with user).

Each site gets a **separate `public_domains` list**, distinct from its local
domains. A public domain is served over **HTTP-only on port 80, with no 301
redirect, no `/etc/hosts` entry, and no mkcert certificate**. TLS is handled
entirely by the upstream public server, which forwards `Host` and
`X-Forwarded-Proto: https`.

Local `.dev` domains keep their existing behaviour (HTTPS + mkcert + hosts)
completely unchanged.

## Components

### 1. Data model — `core/src/site_registry.rs`

- New struct:
  ```rust
  pub struct SitePublicDomains { pub name: String, pub domains: Vec<String> }
  ```
- New field on `SiteRegistry`: `#[serde(default)] public_domains: Vec<SitePublicDomains>`.
- New methods:
  - `public_domains_for(&self, name: &str) -> Option<&[String]>`
  - `set_public_domains(&mut self, name: &str, domains: &[String]) -> Result<(), RegistryError>`
    — normalize (trim + lowercase), dedupe, `validate_domain` each, reject empty
    (`NoDomains`).
  - `remove()` also clears `public_domains` for the name and counts toward the
    "was anything removed" tally.
- **Global uniqueness:** a domain string may appear in at most one place across
  all sites' local `domains` **and** `public_domains`. `set_public_domains` and
  the existing `set_domains` both enforce this, returning `DomainTaken`.

### 2. Site model — `core/src/sites.rs`

- `Site` gains `pub public_domains: Vec<String>` (default empty).
- `list_all_sites` fills `public_domains` from `registry.public_domains_for(name)`
  for every site (scanned, linked, proxy).

### 3. Nginx vhost — `core/src/sites.rs::vhost_config`

- Local-domain output is unchanged (`80→301` + `443 ssl` block).
- When `public_domains` is non-empty, append **one additional HTTP server block**:
  ```nginx
  server {
    listen 80;
    server_name <public domains joined by space>;
    # PHP site:
    root <document_root>;
    index index.php index.html;
    location ~ /\.(?!well-known).* { deny all; }
    <build_cache if laravel>
    location / { try_files $uri $uri/ /index.php?$query_string; }
    location ~ \.php$ {
      fastcgi_pass unix:<sock>;
      fastcgi_index index.php;
      include <nginx_etc>/fastcgi_params;
      fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
      fastcgi_param HTTPS $lara_fwd_https;   # derived from X-Forwarded-Proto
    }
    access_log <name>-public-access.log;
    error_log  <name>-public-error.log;
  }
  ```
- **Proxy-site** (upstream Node/Vite): the public block mirrors the proxy routes
  with `proxy_pass http://<upstream>`, same `proxy_set_header` set (including
  websocket upgrade headers when enabled), but over plain HTTP with no redirect.
- No TLS directives, no `return 301` in the public block.

### 4. `nginx.conf` http{} — `core/src/service/nginx.rs`

- Add one map alongside the existing `$connection_upgrade` map:
  ```nginx
  map $http_x_forwarded_proto $lara_fwd_https { default ''; https on; }
  ```
  So a request the upstream marked `X-Forwarded-Proto: https` sets
  `fastcgi_param HTTPS on`, letting Laravel generate correct `https://` URLs and
  treat the request as secure. Direct HTTP hits (no header) leave it empty.

### 5. Sync — `core/src/sync.rs`

- Certs: `ensure_cert` is still called with **only the site's local `domains`**;
  `public_domains` are never passed to mkcert.
- Hosts: the `explicit` list that feeds `/etc/hosts` is built from local
  `domains` only. `public_domains` are excluded entirely (no `127.0.0.1` pin).

### 6. Command layer — `src-tauri/src/commands.rs`

- New command `set_site_public_domains(name, domains)` mirroring
  `set_site_domains`: load registry, `set_public_domains`, save, re-sync, return
  the same result shape.

### 7. UI — `src/`

- New IPC binding `setSitePublicDomains(name, domains)` in `src/ipc/commands.ts`.
- New modal (or a second section within the Domains modal) **"Public domains"**:
  editable rows, add/remove, client-side `validDomain` check, calls the new
  command. Empty list clears the site's public domains.
- Row action menu: add a **"Public domains"** entry.
- Site card: a badge distinguishing public domains from local ones.
- State (`src/state.ts`): a `sitePublicDomains` editor slice analogous to
  `siteDomains`.

### 8. Docs

- A short guide (README or `docs/`) covering the upstream side:
  - Sample public-server nginx snippet:
    ```nginx
    location / {
      proxy_pass http://<device-ip>:80;
      proxy_set_header Host $host;
      proxy_set_header X-Real-IP $remote_addr;
      proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
      proxy_set_header X-Forwarded-Proto https;
    }
    ```
  - Note that the Laravel app must configure `TrustProxies` (trust the upstream)
    so `X-Forwarded-*` are honoured.

## Data flow

1. User adds `app.example.com` as a public domain on a site via the new modal.
2. `set_site_public_domains` → `registry.set_public_domains` (validate + unique) →
   save `sites.toml` → `sync_sites`.
3. `sync_sites` writes `<name>.conf` containing the unchanged local HTTPS block
   **plus** the new HTTP public block. `/etc/hosts` and mkcert are untouched by
   the public domain.
4. Upstream public server terminates TLS and proxies HTTP + `X-Forwarded-Proto:
   https` to device:80. Nginx routes by `Host`, serves the app, PHP sees
   `HTTPS=on`.

## Error handling

- Invalid domain → `RegistryError::InvalidDomain` (surfaced as command error).
- Empty list submitted → treated as "clear public domains" in the command layer
  (not an error), OR `NoDomains` if the UI always sends ≥1 — decide in plan;
  default: empty submission clears.
- Domain already used by another site (local or public) → `DomainTaken`.
- All registry writes are followed by a re-sync; sync errors propagate to a toast.

## Testing

- `site_registry.rs`: set/get/remove public domains; normalization + dedupe;
  `validate_domain` rejection; cross-list uniqueness (local vs public) both
  directions; old `sites.toml` without `public_domains` still loads.
- `sites.rs`: `Site.public_domains` populated; `vhost_config` emits an HTTP
  public block with `server_name`, no `return 301`, no `listen 443`, and
  `fastcgi_param HTTPS $lara_fwd_https`; proxy-site public block emits
  `proxy_pass` over HTTP; a site with no public domains emits no extra block.
- `sync.rs`: a public domain is absent from the `/etc/hosts` write and absent
  from the names passed to the cert issuer, while local domains still appear.
- `service/nginx.rs`: generated `nginx.conf` contains the
  `$http_x_forwarded_proto` map.

## Out of scope (YAGNI)

- ACME / real Let's Encrypt certificates on the device.
- HTTPS termination on the device for public domains.
- Automatic upstream-server configuration (only documented).
- DNS management for public domains.
