# Laralux — nginx multi-version (catalog + version-parameterized install)

**Date:** 2026-06-26
**Status:** Design (follow-on to the Versioned Tool Manager foundation).
**Goal:** Let the Setup → Nginx modal install and switch between multiple nginx versions, by
filling in the foundation seam (`tools::available_versions` / `tools::install_version`) for nginx —
mirroring how PHP works. No UI changes needed (the modal already consumes the seam).

---

## 1. Context

The foundation (merged) gives a uniform registry + Setup modal + symlink toggle, with **PHP** as the
only multi-version tool. `tools::available_versions(tool)` returns a static known-list ∪ installed for
PHP, and installed-only for every other tool; `tools::install_version` dispatches PHP→`install_php_static`
and returns `Unsupported` for the rest. `Orchestrator::replace_version(kind, tool, version)` already
generalizes version switching.

nginx binaries come from `jirutka/nginx-binaries`. Verified June 2026: the x86_64/linux index lists 42
versions; filenames follow the exact pattern `nginx-{version}-{arch}-linux` (e.g.
`nginx-1.31.2-x86_64-linux`), so a specific version installs by constructing the filename — **no index
fetch needed** for a version-targeted install. `run_setup` already records `config.versions["nginx"]`.

## 2. Approach

Add a curated `KNOWN_NGINX_VERSIONS` list (latest patch of recent minor lines) like PHP's, and a
version-targeted installer that builds the filename directly. Wire nginx into the registry's
`available_versions` (known ∪ installed) and `install_version`. Switching is already generic via
`replace_version`, with one nginx-specific wrinkle in the desktop command: the new binary file needs
`cap_net_bind_service` re-applied (`ensure_nginx_bind_cap`) **after** `current` is repointed and
**before** the service starts — so nginx's switch path in `set_tool_version` does
stop → set_current → setcap → start instead of the plain `replace_version`.

## 3. Components

### 3.1 `core/src/nginx_static.rs`
- `pub const KNOWN_NGINX_VERSIONS: [&str; 6] = ["1.31.2", "1.30.3", "1.29.8", "1.28.3", "1.27.5", "1.26.3"];`
  (all verified present in the live index; latest patch per recent minor.)
- `pub fn nginx_filename(version: &str, arch: &str) -> String` → `format!("nginx-{version}-{arch}-linux")`.
- `pub fn install_nginx_version(paths, version, downloader, sink) -> Result<String, NginxError>`:
  arch-gate; `dest = bin/nginx/<version>/nginx`; idempotent (if installed → `set_current` + return);
  download `{NGINX_BASE_URL}/{nginx_filename(version, arch)}` → chmod 0755 → atomic temp→rename →
  `set_current(paths, "nginx", version)`; return version. A 404 (unknown version) surfaces as
  `NginxError::Download`.
- Refactor the shared download→chmod→rename→set_current block out of `install_nginx` into a private
  `place_nginx(paths, version, url, downloader, sink)` reused by both `install_nginx` (latest, via the
  index — unchanged behavior for `run_setup`) and `install_nginx_version`.

### 3.2 `core/src/tools.rs`
- `available_versions`: add a `Nginx` arm → `KNOWN_NGINX_VERSIONS` ∪ `installed_versions(paths,"nginx")`,
  `active = config.versions["nginx"]`, deduped and version-sorted descending (same shape PHP produces).
  Other non-PHP tools keep the installed-only arm.
- `install_version`: add a `Nginx` arm → `install_nginx_version(...)`; PHP unchanged; others `Unsupported`.

### 3.3 `src-tauri/src/commands.rs` — `set_tool_version`
- Special-case `ManagedTool::Nginx`: read running state + stop under one lock; `set_current(paths,"nginx",full)`;
  `ensure_nginx_bind_cap(paths, &PkexecPrivileged)`; re-lock and `start(Nginx)` iff it had been running;
  snapshot. Non-nginx tools keep the existing `replace_version` / `set_current` branch.

## 4. Behavior & errors
- Best-effort, idempotent installs; failures surface as `Err(String)` toasts in the UI.
- The Setup → Nginx modal now lists the known versions with Install/Use/Active and the existing
  `/usr/local/bin` `nginx` symlink toggle; switching a running nginx re-applies the bind capability so it
  binds :80/:443 cleanly (pkexec prompt only when setcap is actually needed — `ensure_nginx_bind_cap`
  self-skips via `getcap`).

## 5. Testing
- `nginx_static`: `nginx_filename` exact string; `install_nginx_version` idempotent + writes to
  `bin/nginx/<version>/nginx` + sets `current` (stub downloader). Live-verify a real version download.
- `tools`: `available_versions(Nginx)` includes the known set (≥6) and marks an installed/active entry;
  `install_version(Nginx, ...)` no longer returns `Unsupported`. Update the existing
  `single_version_tool_lists_installed_only` test to use a still-single-version tool (Mariadb) since
  nginx is now multi-version.
- Build `-p laralux-desktop` (the `set_tool_version` nginx branch). Live: install + switch nginx in the app.

## 6. Out of scope
- aarch64 curation (pattern maps; x86_64 verified).
- Other tools' catalogs (mariadb/redis/mailpit/mkcert/composer) — each its own follow-on.
