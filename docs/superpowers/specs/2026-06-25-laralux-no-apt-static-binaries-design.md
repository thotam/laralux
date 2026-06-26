# Laralux — No-apt Static Binaries (Spec 1: mkcert + composer + nginx + redis/Valkey) Design

**Date:** 2026-06-25
**Status:** Design approved, pending spec review
**Goal:** Make `run_setup` install mkcert, composer, nginx, and redis as **downloaded binaries into `~/laralux/bin`** (no `sudo apt`), so the stack is self-contained and distro-independent — matching how PHP, mailpit, and CoreDNS are already installed. MariaDB is deferred to **Spec 2** (heavier: large tarball + datadir init).

---

## 1. Context & current state

`core::setup::run_setup` installs the missing core stack via `Privileged::apt_install(apt_packages_for(component))`:

- **nginx** → apt `nginx`
- **mariadb** → apt `mariadb-server` (out of scope here — Spec 2)
- **redis** → apt `redis-server`
- **mkcert** → apt `mkcert` + `libnss3-tools`
- **composer** → apt `composer`

PHP (`php_static.rs`), mailpit (download in `run_setup`), and CoreDNS (`coredns.rs`) are already downloaded (no apt). The download pattern is established: fetch via `Downloader`, extract/install a single binary into `paths.bin()`, chmod 0755, idempotent when already present.

`detect(paths)` marks a component **present** when its binary resolves under `[paths.bin()]` (or PATH): `nginx`, `redis-server`, `mkcert`, `composer`. The orchestrator resolves program names via `bin::resolve_or_name(name, &[paths.bin()])` — `~/laralux/bin` first — so installing under these exact names makes the app-managed processes pick them up with no service changes.

`Privileged::install_mkcert_ca` currently runs `mkcert -install` resolved from **PATH** (un-escalated). `setcap_nginx(bin)` already takes a resolved binary path. `install_composer(paths, downloader)` already exists: it downloads `composer.phar` + writes a `composer` wrapper that runs it under `~/laralux/bin/php` — but `run_setup` does NOT call it (composer comes from apt today).

## 2. Approach (chosen: download static binaries, mirroring `php_static`/`coredns`)

Add one focused core module per component (except composer, which already has `install_composer`). Each follows the existing download idiom and is wired into `run_setup`, replacing the apt path for that component. Sources verified current (June 2026):

| Component | Source (no apt) | Currency |
|-----------|-----------------|----------|
| mkcert | `github.com/FiloSottile/mkcert` release binary | v1.4.4 (stable, single binary) |
| composer | `getcomposer.org/composer.phar` (existing `install_composer`) | always latest phar |
| nginx | `jirutka/nginx-binaries` static (musl + openssl 3.x), via `index.json` | 1.31.2 (2026-06-21) |
| redis | **Valkey** official tarball (redis-server-compatible fork) | 9.1.0 (2026-05-19) |

**Rejected:**
- Third-party static **redis** builds (`phlummox-dev/redis-static-binaries`): stale (6.2.5, ~2021). Valkey is a drop-in, actively-maintained fork with current official binaries.
- Building nginx/redis from source at setup: needs a toolchain (gcc/make) that itself usually means apt — defeats the goal.
- Switching nginx→Caddy: jirutka's static nginx is current (1.31.2) and keeps the existing nginx vhost generation unchanged; no rewrite needed.

## 3. Architecture & components

All new modules live in `core` (zero Tauri deps), are re-exported from `lib.rs`, and use `Downloader` (+ `CommandRunner` for tar) seams so the pure URL/parse logic is unit-tested and the download/extract is verified live.

### 3.1 `core/src/mkcert_static.rs` (new)

- `pub const MKCERT_VERSION: &str = "1.4.4";`
- `pub fn mkcert_arch() -> Option<&'static str>` — `x86_64 → "amd64"`, `aarch64 → "arm64"`, else `None`.
- `pub fn mkcert_url(version: &str, arch: &str) -> String` →
  `https://github.com/FiloSottile/mkcert/releases/download/v{version}/mkcert-v{version}-linux-{arch}`.
- `pub fn install_mkcert(paths: &LaraluxPaths, downloader: &dyn Downloader) -> Result<(), MkcertError>`:
  - dest = `paths.bin().join("mkcert")`. Idempotent: return `Ok(())` if a non-empty file already exists (reuse the `coredns_installed`-style check: non-empty regular file).
  - else map arch (`MkcertError::Arch` if unsupported), `create_dir_all(paths.bin())`, download the binary to a temp path under `paths.tmp()`, chmod 0755, atomically `rename` into `bin/mkcert` (copy+remove cross-device fallback) — same atomic pattern as `ensure_coredns`.
- `pub enum MkcertError` (thiserror): `Arch(String)`, `Download(String)`, `Io(#[from] std::io::Error)`.

### 3.2 `core/src/nginx_static.rs` (new)

- `pub const NGINX_INDEX_URL: &str = "https://jirutka.github.io/nginx-binaries/index.json";`
- `pub const NGINX_BASE_URL: &str = "https://jirutka.github.io/nginx-binaries";` (the binary is served at `{base}/{filename}`).
- `pub fn nginx_arch() -> Option<&'static str>` — `x86_64 → "x86_64"`, `aarch64 → "aarch64"`, else `None`.
- `pub fn latest_nginx_filename(arch: &str, index_json: &str) -> Option<String>` (pure, unit-tested): parse the index (array of `{ name, version, arch, os, filename, ... }`); among entries with `name == "nginx"`, `os == "linux"`, `arch == <arch>`, pick the one whose `version` is highest (semver-ish: split on `.`, compare numeric components) and return its `filename`. Mirrors `php_static::latest_patch_url`'s "highest match wins" intent.
- `pub fn install_nginx(paths, downloader, runner) -> Result<(), NginxError>`:
  - dest = `paths.bin().join("nginx")`. Idempotent on non-empty file.
  - map arch; fetch the index JSON to `paths.tmp()/nginx-index.json`, read it, `latest_nginx_filename(...)` → `NginxError::NoBuild` if none; download `{NGINX_BASE_URL}/{filename}` to a temp path; chmod 0755; atomic rename into `bin/nginx`.
- `pub enum NginxError` (thiserror): `Arch(String)`, `NoBuild`, `Download(String)`, `Parse(String)`, `Io(#[from] std::io::Error)`.
- The binary is invoked by the existing `NginxService` exactly as before (`nginx -p <etc/nginx> -c <conf>` overrides the compiled-in prefix), so no service change is needed. The generated `nginx.conf` is self-contained — it uses absolute `include` paths and `default_type application/octet-stream;` and does **not** reference `mime.types` or any distro prefix — so a prefix-less static binary behaves identically (verified against `core/src/service/nginx.rs`). The static build bundles `http_ssl`, `http_v2`, `http_realip`, and `stream` — all the directives the generated vhosts use.

> **Note on the index parser:** keep `latest_nginx_filename` tolerant — ignore entries missing fields or with non-numeric version parts rather than failing the whole parse. Use `serde_json` (already a `laralux-core` dependency: `Cargo.toml` lists `serde_json = "1"`).

### 3.3 `core/src/redis_static.rs` (new) — Valkey

- `pub const VALKEY_VERSION: &str = "9.1.0";`
- `pub fn valkey_arch() -> Option<&'static str>` — `x86_64 → "x86_64"`, `aarch64 → "arm64"`, else `None`.
- `pub fn valkey_url(version: &str, arch: &str) -> String` →
  `https://download.valkey.io/releases/valkey-{version}-jammy-{arch}.tar.gz` (the **jammy** glibc base maximizes cross-distro compatibility).
- `pub fn install_redis(paths, downloader, runner) -> Result<(), RedisError>`:
  - dest = `paths.bin().join("redis-server")`. Idempotent on non-empty file.
  - map arch; download the tarball to `paths.tmp()/valkey.tar.gz`; extract into a unique temp dir under `paths.tmp()` (e.g. `valkey-extract/`); locate `valkey-server` inside the extracted tree (the tarball nests under `valkey-{version}-jammy-{arch}/bin/valkey-server` — find it robustly by walking for a file named `valkey-server`); chmod 0755; atomically install it as `bin/redis-server` (rename, copy+remove fallback). Valkey is a drop-in fork: `valkey-server` reads the same `redis.conf` and CLI flags the existing `RedisService` generates, so installing it under the name `redis-server` needs no service change.
  - (Optional, same extraction: also install `valkey-cli` as `bin/redis-cli` for parity. Include it — it's free and users expect a CLI.)
- `pub enum RedisError` (thiserror): `Arch(String)`, `Download(String)`, `Extract(String)`, `Io(#[from] std::io::Error)`.

### 3.4 `Privileged::install_mkcert_ca` — use the downloaded binary

Change the trait method to take the resolved mkcert path so the CA is installed with the binary we just downloaded (not a PATH lookup that may miss `~/laralux/bin`):

- `fn install_mkcert_ca(&self, mkcert_bin: &Path) -> Result<(), PrivError>;`
- `SudoPrivileged`/`PkexecPrivileged`: run `<mkcert_bin> -install` (un-escalated, as today — `mkcert -install` installs the CA into the user's NSS/trust stores; this preserves current behavior, only the binary path changes). `FakePrivileged`: record the call.
- `run_setup` passes `resolve_bin("mkcert", &[paths.bin()])` (skip the CA step with a recorded error if mkcert didn't install).

### 3.5 `core/src/setup.rs` — wire into `run_setup`, drop apt

- `apt_packages_for`: nginx/redis/mkcert/composer now return **empty** (only mariadb keeps `["mariadb-server"]` until Spec 2). Keep the function (mariadb still uses it).
- In `run_setup`, after PHP-static and before mailpit, install each missing component via its new module (best-effort, push errors to `report.errors`):
  - `Component::Mkcert` missing → `install_mkcert(paths, downloader)`.
  - `Component::Nginx` missing → `install_nginx(paths, downloader, runner)`.
  - `Component::Redis` missing → `install_redis(paths, downloader, runner)`.
  - `Component::Composer` missing → `install_composer(paths, downloader)` (existing fn).
- The apt block stays but now only ever receives mariadb's package (until Spec 2 removes it). `disable_system_services` and `allow_mariadb_apparmor` remain (still relevant for a distro mariadb / leftover units).
- `SetupReport`: add booleans `mkcert_fetched`, `nginx_fetched`, `redis_fetched`, `composer_fetched` (mirror `mailpit_fetched`) so the UI/CLI can report what was downloaded. The mkcert CA step now reads `install_mkcert_ca(&resolved_mkcert)`.

### 3.6 `lib.rs` re-exports

Re-export the new entry points + error types: `install_mkcert`, `MkcertError`; `install_nginx`, `NginxError`; `install_redis`, `RedisError`. (`install_composer` is already re-exported.)

## 4. Behavior, currency & error handling

- **Self-contained:** after setup, `~/laralux/bin` holds `nginx`, `redis-server` (+`redis-cli`), `mkcert`, `composer`(+`composer.phar`), alongside the existing `php*`, `mailpit`, `coredns`. No distro packages required; works on non-Debian distros.
- **Currency:** nginx tracks the index's latest (1.31.2 today, auto-follows new builds); Valkey/mkcert are version-pinned consts (bump by editing one line, like `COREDNS_VERSION`).
- **Idempotent:** every installer skips when its non-empty binary already exists; re-running setup is cheap and prompt-free.
- **Best-effort:** a download failure for one component pushes an error into `report.errors` and does not abort the others (matches today's mailpit/PHP handling).
- **No new root:** all four are user-space downloads. The only privileged steps remain the existing ones (mkcert CA is un-escalated; `setcap nginx`, `/etc/hosts`, resolved drop-in unchanged).

## 5. Testing (TDD; no network in unit tests)

- `mkcert_url`/`mkcert_arch`: exact URL string + arch mapping.
- `nginx_arch`; `latest_nginx_filename`: feed a small JSON fixture with several nginx entries (mixed arch/os/version) → returns the highest-version linux/<arch> `filename`; ignores non-matching/malformed entries; `None` when no match.
- `valkey_url`/`valkey_arch`: exact URL + arch mapping.
- Installers (`install_*`) shell out / extract and are **verified live** (depend on the host + network), like `ensure_coredns` and the mailpit fetch; the pure functions carry the unit coverage. A `FakeDownloader`/`FakeCommandRunner` test may assert the install attempts the expected URL when the binary is absent (mirroring existing `php_static`/`setup` tests).
- `Privileged::install_mkcert_ca`: `FakePrivileged` records the passed path.
- `run_setup`: existing tests use `FakePrivileged`/`FakeDownloader`/`FakeCommandRunner`; update them so a missing component triggers the new installer path (and apt is no longer expected for nginx/redis/mkcert/composer). Assert `apt_packages_for(Nginx|Redis|Mkcert|Composer)` is empty and `apt_packages_for(Mariadb)` still has the package.

## 6. Out of scope (Spec 2 / backlog)

- **MariaDB** static tarball (mariadb.org glibc build) + datadir init via `mariadb-install-db` + support files — its own spec (heaviest piece).
- Removing the `Privileged::apt_install`/`add_apt_repository` trait methods entirely (still used by mariadb until Spec 2; remove then).
- A Settings UI to pin/override component versions.
- Firefox/NSS CA trust beyond what user-level `mkcert -install` covers (the apt `libnss3-tools`/`certutil` dependency is dropped; document that Firefox trust may need `certutil` present, otherwise Chromium/system trust still works).
- ARM/macOS builds beyond the arch mappings above (mappings are included but only x86_64 Linux is the primary verified target).
