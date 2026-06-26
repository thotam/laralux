# Laralux Linux — Custom Domains per Site (Phase 2/3) Design Spec

**Date:** 2026-06-25
**Status:** Design approved, pending spec review
**Goal:** Let the user edit a site's domains: add extra domains and explicit subdomains, change/remove the auto primary `<name>.<tld>`, and add **wildcard** subdomains (`*.demo.dev`) — for every site type (scanned/linked/proxy). Each explicit domain is wired into the site's TLS cert (SAN), `/etc/hosts`, and nginx `server_name`; wildcard domains are resolved by an **app-managed CoreDNS** (a downloaded static binary, no apt) routed in via a systemd-resolved drop-in.

This is a single, undivided feature (the user requested no split). The wildcard/DNS part is the system-level, higher-risk component; explicit domains work independently of it.

---

## 1. Context & current state

Each site has one auto-derived hostname `<name>.<tld>` (`core::sites::Site.hostname`). `Site::vhost_config` writes `server_name <hostname>`; `core::sync::sync_sites` issues one mkcert cert per hostname (`CertIssuer::ensure_cert(hostname)` → `<hostname>.pem`) and writes one `/etc/hosts` line per hostname (via `hosts::apply_block`). Sites come from `list_all_sites` (scanned `www/` dirs + registry `[[sites]]`/`[[proxies]]`). The host uses **systemd-resolved** (stub `127.0.0.53`) with NetworkManager in default mode. `/etc/hosts` cannot express wildcards, so wildcard subdomains require a real local DNS resolver.

The project already downloads static binaries into `~/laralux/bin` and runs them via the orchestrator (mailpit; the static php-fpm/php). The same pattern is used here for the DNS resolver — **no apt**.

## 2. Approach (chosen: per-site domain override + SAN cert + hosts/CoreDNS split)

Persist a per-site domain list (keyed by site name, overriding the default) in `sites.toml`. A site's effective domains drive a single multi-SAN cert, the `server_name` list, `/etc/hosts` (explicit names), and — for wildcard names — an **app-managed CoreDNS** (downloaded static binary) plus a systemd-resolved routing drop-in (only the laralux domains route to it; the system's default DNS is untouched). An Edit-domains modal manages the list for any site.

Rejected:
- **dnsmasq via apt** — the user wants a downloaded static binary, not an apt install; dnsmasq has no canonical static release. **CoreDNS** ships official single-file static binaries (GitHub releases, with sha256) and answers wildcard zones via its `template` plugin — a clean drop-in match for the existing mailpit/php download model.
- **NetworkManager `dns=dnsmasq` mode** — invasive on a systemd-resolved host. The app-managed resolver + a resolved routing drop-in is reversible and scoped to laralux domains.
- **Wildcard via `/etc/hosts`** — impossible (no wildcards).
- **Splitting wildcard into a later slice** — the user wants it together.

## 3. Architecture & components

### 3.1 `core/src/site_registry.rs` — per-site domains

- `struct SiteDomains { name: String, domains: Vec<String> }` (serde); `SiteRegistry` gains `#[serde(default)] domains: Vec<SiteDomains>` (TOML `[[domains]]`).
- `pub fn validate_domain(d: &str) -> Result<(), RegistryError>`: accept a DNS hostname of 1+ dot-separated labels (each `^[a-z0-9]([a-z0-9-]*[a-z0-9])?$`), or a wildcard `*.<rest>` where the leftmost label is exactly `*` and `<rest>` is a valid hostname. Reject empty/space/uppercase/invalid/multi-`*`.
- Methods:
  - `set_domains(&mut self, name: &str, domains: &[String]) -> Result<(), RegistryError>`: require ≥1 domain (`NoDomains`); validate each; reject duplicates within the list and **across all other sites' effective domains** (`DomainTaken`); store (replacing any existing entry for `name`).
  - `domains_for(&self, name: &str) -> Option<&[String]>`: the override list for a site, if any.
  - `remove(&mut self, name)` (existing) also drops the matching `[[domains]]` entry.
- New `RegistryError` variants: `InvalidDomain(String)`, `NoDomains`, `DomainTaken(String)`.

### 3.2 `core/src/sites.rs` — effective domains on `Site`

- `Site` gains `pub domains: Vec<String>` (effective, non-empty). `hostname` stays = `domains[0]` (display/URL/cert basename).
- `list_all_sites`: for each site, `domains = registry.domains_for(name)` if present, else `vec!["<name>.<tld>"]`; set `hostname = domains[0]`. `scan_sites` sets `domains: vec![hostname.clone()]`.
- `Site::vhost_config`: `server_name` = all `self.domains` space-joined (works for both PHP and proxy branches; nginx accepts `*.demo.dev` and exact names together).

### 3.3 `core/src/ssl.rs` — multi-SAN cert

- `CertIssuer::ensure_cert(&self, basename: &str, names: &[String]) -> Result<CertFiles, SslError>` (signature change): issue **one** cert (`mkcert <names...>`) covering all `names`, written as `<basename>.pem`/`<basename>-key.pem` (basename = a filesystem-safe form of the primary, `*` → `_wildcard`). Write a `<basename>.san` sidecar listing the sorted names; re-issue when the cert/key/sidecar is missing **or** the sidecar's names differ. `FakeCertIssuer` records `(basename, names)`.
- Update `sync_sites` and any caller to pass `(basename, names)`.

### 3.4 `core/src/sync.rs` — fan out cert/hosts/wildcard

- For each site: `names = site.domains`; `basename = cert_basename(&site.name)`; `issuer.ensure_cert(&basename, &names)`; vhost as today (server_name now multi).
- `/etc/hosts` block = every **non-wildcard** domain across all sites (→ `127.0.0.1`).
- Collect the set of **wildcard bases** (`*.X` → `X`) across all sites; return them so the IPC layer can apply the DNS step. `sync_sites` returns `SyncOutcome { sites: Vec<Site>, warnings: Vec<String>, wildcard_bases: Vec<String> }` (replacing the current `(Vec<Site>, Vec<String>)` tuple); update all callers.

### 3.5 `core/src/coredns.rs` (new) + a `Coredns` service — wildcard resolver

- `const COREDNS_VERSION`, `coredns_url(version, arch) -> String` (pure): `https://github.com/coredns/coredns/releases/download/v<ver>/coredns_<ver>_linux_<arch>.tgz` (`arch ∈ {amd64, arm64}`).
- `ensure_coredns(paths, downloader, runner) -> Result<(), CorednsError>`: if `~/laralux/bin/coredns` is missing, download the tgz to `tmp/`, `tar -xzf … -C bin coredns`, chmod 0755 (the tgz contains a single `coredns` binary). Reuses the existing `Downloader`/`CommandRunner` seams — **no apt, no root**.
- `corefile(bases: &[String], port: u16) -> String` (pure): one zone block per base, e.g.
  ```
  demo.dev:5353 {
      template IN A { answer "{{ .Name }} 60 IN A 127.0.0.1" }
      template IN AAAA { rcode NXDOMAIN }
  }
  ```
  Port `5353`, bind `127.0.0.1`.
- A `Coredns` service in `core::service`: writes `etc/coredns/Corefile` from `corefile`, spawns `coredns -conf <Corefile>` (resolved from `~/laralux/bin`). It is started by `start_all` **only when** wildcard bases exist; otherwise stopped/absent.
- `enum CorednsError` (thiserror): `Arch`, `Download`, `Extract`, `Io`.

### 3.6 `core/src/privileged.rs` — systemd-resolved drop-in

- `write_resolved_dropin(&self, contents: &str) -> Result<(), PrivError>`: write `/etc/systemd/resolved.conf.d/laralux.conf` + `systemctl reload systemd-resolved`.
- `remove_resolved_dropin(&self) -> Result<(), PrivError>`: delete it + reload.
- `resolved_dropin(bases, port) -> String` (pure, in `coredns.rs`): `[Resolve]\nDNS=127.0.0.1:<port>\nDomains=` + `~<base>` per base (routing-only domains, so only these route to CoreDNS; the default DNS is untouched). Both privileged ops via pkexec/sudo, reversible.

### 3.7 IPC (Tauri) — `src-tauri/src/commands.rs`

- `set_site_domains(app, name: String, domains: Vec<String>) -> Result<Vec<Site>, String>` (async): load registry → `set_domains` (validation → toast) → save → `sync_sites` (cert/hosts) → if `wildcard_bases` non-empty: `ensure_coredns` (download if missing) + start/refresh the `Coredns` service with the new Corefile + `write_resolved_dropin(resolved_dropin(bases))`; if empty: stop CoreDNS + `remove_resolved_dropin` → reload nginx → return refreshed sites. DNS failures are non-fatal warnings.
- `stack_start_all` also starts CoreDNS when wildcard bases exist (so it survives a full restart). Register `set_site_domains` in `main.rs`.

### 3.8 Frontend — `dist/` (Edit-domains modal)

- Add an **Edit domains** action to **every** site row (reuse the proxy modal's `ns-*` tokens/a11y).
- Modal: a dynamic list of domain rows (add/remove), each a text input; the first row is the primary but fully editable; wildcard (`*.x`) allowed; live validation mirroring `validate_domain`; a note that wildcard uses the bundled DNS helper (a pkexec prompt may appear). Submit → `invoke("set_site_domains", { name, domains })`; success toast; error toast keeps the modal open. No `alert()`.
- The existing proxy **Edit** (routes) stays separate; domains Edit is an additional action.

## 4. Behavior details & decisions

- **Primary editable/removable**: the override list fully replaces the default, so the user can rename or drop `<name>.<tld>`. The URL/Open button uses `domains[0]`.
- **Uniqueness**: a domain may belong to only one site (validated at write across all sites' effective domains).
- **Explicit vs wildcard**: explicit → `/etc/hosts`; wildcard `*.X` → CoreDNS zone `X` (covers `X` and all sub-levels). Both go into the cert SAN.
- **CoreDNS lifecycle**: downloaded on demand into `~/laralux/bin`; runs only while some site has a wildcard domain; its Corefile + the resolved drop-in are regenerated on every domain change; both removed when no wildcard remains.
- **No system-DNS takeover**: the drop-in adds *routing* domains (`~base`) pointing at CoreDNS; all other names keep using the existing resolver. Fully reversible by deleting the drop-in.
- **No apt / no root for the binary**: CoreDNS lives under `~/laralux` (user-owned). Only the resolved drop-in write/reload needs pkexec.

## 5. Error handling

- Domain validation (invalid, none, taken) → typed `RegistryError` → `Err(String)` → modal toast; modal stays open.
- Cert/hosts/nginx failures: best-effort like the existing `sync_sites` path.
- CoreDNS/resolved failures (download, extract, write, reload) are **non-fatal**: collected as warnings; explicit domains remain functional.
- Removing all wildcard domains cleans up the drop-in + stops CoreDNS; a failure there is a warning, not fatal.

## 6. Testing (TDD; fakes only, no network/DNS/root)

- `validate_domain`: accepts `app2.dev`, `api.demo.dev`, `my-app.local`, `*.demo.dev`; rejects ``, `Demo.dev`, `a b.dev`, `*.*.dev`, `*x.dev`, `foo.*.dev`.
- registry `set_domains`: rejects empty (`NoDomains`), invalid domain, a domain already used by another site (`DomainTaken`); stores + `domains_for` round-trips; `remove` drops the `[[domains]]` entry; old `sites.toml` without `[[domains]]` still loads.
- `list_all_sites`: a site with an override exposes those `domains` and `hostname == domains[0]`; without an override, `domains == ["<name>.<tld>"]`.
- `Site::vhost_config`: `server_name` contains all domains incl. a `*.demo.dev`; proxy + php branches both.
- `ensure_cert(basename, names)`: requests one cert with all names; `.san` sidecar drives re-issue when names change.
- `sync_sites`: hosts block contains explicit domains only; `wildcard_bases` returned (not in hosts); cert requested per site with its full name set.
- `coredns`: `coredns_url("1.14.4","amd64")` is the expected GitHub URL; `corefile(["demo.dev"],5353)` contains `demo.dev:5353` + the `template IN A` answer line; `resolved_dropin(["demo.dev","test"],5353)` contains `DNS=127.0.0.1:5353` and `Domains=~demo.dev ~test`; `ensure_coredns` with module-local fakes downloads the tgz + extracts `coredns` to `bin/` (mode 0755) only when missing.
- Live-only (human-verified): mkcert wildcard issuance, CoreDNS download + spawn, resolved drop-in + reload, browser resolution of `*.demo.dev`.

## 7. Out of scope (backlog)

- Renaming a site (the directory/registry key); changing a site's `root`.
- Choosing a different local DNS mechanism (dnsmasq, NetworkManager); only the CoreDNS + systemd-resolved drop-in path is implemented.
- IPv6 (`::1`) hosts/wildcard answers.
- Multi-level wildcards (`*.*.demo.dev`).
- Per-domain TLS/HSTS settings.
