# Laralux — No-apt Spec 1 (nginx + redis/Valkey + mkcert) into the versioned layout

**Date:** 2026-06-26
**Status:** Design (goal-directed); supersedes the 2026-06-25 Spec-1 draft (which predated the versioned-binary layout).
**Goal:** Install **nginx**, **redis (Valkey)**, and **mkcert** as downloaded binaries into the versioned layout (`~/laralux/bin/<tool>/<version>/` + `current` symlink), removing them from `apt`. Composer/PHP/mailpit/coredns are already downloaded. Only MariaDB remains on apt afterward (Spec 2).

---

## 1. Context & current state

After Spec 0 (versioned layout), downloaded tools live in `bin/<tool>/<version>/<binary>` with a config-driven `bin/<tool>/current` symlink; `config.versions` (tool→version) is the source of truth; `apply_versions` reconciles the symlinks. Installers (php_static, coredns, the mailpit/composer paths in `run_setup`) follow this: download → (extract) → atomic temp→rename into the version dir → `layout::set_current` → record `config.versions[tool]`. Each download-bearing fn takes a `sink: &dyn ProgressSink` (Spec: download progress).

`apt_packages_for` still returns apt packages for **Nginx** (`nginx`), **Redis** (`redis-server`), **Mkcert** (`mkcert`,`libnss3-tools`), and **Mariadb** (`mariadb-server`). `run_setup` apt-installs whatever the missing components map to. `detect()` resolves PHP/Composer via `managed_bin_dirs`, but the `other` arm (nginx/redis/mkcert/mariadb/mailpit) still uses `resolve_bin(detect_binary(c), &[paths.bin()])` (flat bin + `$PATH` fallback) — so apt binaries are found via `$PATH`. `Privileged::install_mkcert_ca(&self)` runs `mkcert -install` resolved from `$PATH`.

Verified sources (current, June 2026): nginx static `jirutka/nginx-binaries` (1.31.2, has `index.json`, musl+openssl); Valkey official tarball (9.1.0, ships `valkey-server`/`valkey-cli`, redis-compatible); mkcert GitHub release binary (v1.4.4).

## 2. Approach

Add one core module per tool (`mkcert_static`, `nginx_static`, `redis_static`), each mirroring the existing download idiom and installing into the layout. Wire them into `run_setup`, empty their `apt_packages_for`, fix `detect()` to use `managed_bin_dirs`, and make the mkcert CA install use the downloaded binary. `core` stays Tauri-free; pure URL/parse logic is unit-tested; the download/extract is live-verified.

## 3. Architecture & components

### 3.1 `core/src/mkcert_static.rs` (new)
- `pub const MKCERT_VERSION: &str = "1.4.4";`
- `pub fn mkcert_arch() -> Option<&'static str>` — `x86_64→"amd64"`, `aarch64→"arm64"`, else `None`.
- `pub fn mkcert_url(version, arch) -> String` → `https://github.com/FiloSottile/mkcert/releases/download/v{version}/mkcert-v{version}-linux-{arch}`.
- `pub fn install_mkcert(paths, downloader, sink) -> Result<String, MkcertError>`: idempotent if `bin/mkcert/<MKCERT_VERSION>/mkcert` is a non-empty file (reuse a `coredns_installed`-style check); else map arch (`MkcertError::Arch`), download (`fetch_with_progress`) to a temp path under `paths.tmp()`, chmod 0755, atomic `rename` into `version_dir("mkcert", MKCERT_VERSION).join("mkcert")` (copy+remove cross-device fallback), `layout::set_current(paths,"mkcert",MKCERT_VERSION)`, return the version.
- `pub enum MkcertError` (thiserror): `Arch(String)`, `Download(String)`, `Io(#[from] std::io::Error)`.

### 3.2 `core/src/nginx_static.rs` (new)
- `pub const NGINX_INDEX_URL: &str = "https://jirutka.github.io/nginx-binaries/index.json";`
- `pub const NGINX_BASE_URL: &str = "https://jirutka.github.io/nginx-binaries";`
- `pub fn nginx_arch() -> Option<&'static str>` — `x86_64→"x86_64"`, `aarch64→"aarch64"`, else `None`.
- `pub fn latest_nginx(arch, index_json) -> Option<(String /*version*/, String /*filename*/)>` (pure, unit-tested): parse the JSON array; among entries with `name=="nginx"`, `os=="linux"`, `arch==<arch>`, pick the highest `version` (numeric component compare) → `(version, filename)`. Tolerant: skip malformed/incomplete entries.
- `pub fn install_nginx(paths, downloader, sink) -> Result<String, NginxError>`: idempotent on a non-empty `bin/nginx/<ver>/nginx` for the resolved version — but the version is only known after reading the index; so: map arch; fetch the index JSON to `paths.tmp()`; `latest_nginx(...)` → `NginxError::NoBuild` if none; if `bin/nginx/<ver>/nginx` already non-empty → `set_current` + return ver (idempotent); else download `{NGINX_BASE_URL}/{filename}` (`fetch_with_progress`) to temp, chmod 0755, atomic rename into `version_dir("nginx",&ver).join("nginx")`, `set_current(paths,"nginx",&ver)`, return ver.
- `pub enum NginxError` (thiserror): `Arch(String)`, `NoBuild`, `Download(String)`, `Parse(String)`, `Io(#[from] std::io::Error)`.
- The binary is invoked unchanged by `NginxService` (`nginx -p <etc/nginx> -c <conf>`); the static build bundles `http_ssl`/`http_v2`/`http_realip`/`stream`; the generated `nginx.conf` is self-contained (no `mime.types`/prefix reliance) — verified earlier.

### 3.3 `core/src/redis_static.rs` (new) — Valkey
- `pub const VALKEY_VERSION: &str = "9.1.0";`
- `pub fn valkey_arch() -> Option<&'static str>` — `x86_64→"x86_64"`, `aarch64→"arm64"`, else `None`.
- `pub fn valkey_url(version, arch) -> String` → `https://download.valkey.io/releases/valkey-{version}-jammy-{arch}.tar.gz` (jammy = broad glibc compat).
- `pub fn install_redis(paths, downloader, runner, sink) -> Result<String, RedisError>`: idempotent on a non-empty `bin/redis/<VALKEY_VERSION>/redis-server`; else map arch; download tarball (`fetch_with_progress`) to `paths.tmp()`; extract into a unique temp dir under `paths.tmp()` (tar `-xzf`); locate `valkey-server` (and `valkey-cli`) by walking the extracted tree (tarball nests under `valkey-<ver>-jammy-<arch>/bin/`); chmod 0755; atomically install `valkey-server` as `bin/redis/<ver>/redis-server` and `valkey-cli` as `bin/redis/<ver>/redis-cli` (Valkey is a drop-in fork → `RedisService` (runs `redis-server <conf>`) is unchanged); `set_current(paths,"redis",VALKEY_VERSION)`, return the version.
- `pub enum RedisError` (thiserror): `Arch(String)`, `Download(String)`, `Extract(String)`, `Io(#[from] std::io::Error)`.

### 3.4 `Privileged::install_mkcert_ca` — use the downloaded binary
- Change the trait method to `fn install_mkcert_ca(&self, mkcert_bin: &Path) -> Result<(), PrivError>;` (mirrors `setcap_nginx(bin)`). `SudoPrivileged`/`PkexecPrivileged` run `<mkcert_bin> -install` (un-escalated, as today — installs the user-store CA; behavior unchanged, only the binary path). `FakePrivileged` records the path.
- `run_setup` passes `resolve_bin("mkcert", &managed_bin_dirs(paths))`; if mkcert isn't resolvable, record a note and skip the CA step.

### 3.5 `core/src/setup.rs` — wire in, drop apt, fix detect
- `apt_packages_for`: `Nginx`, `Redis`, `Mkcert` now return **empty**; only `Mariadb` keeps `["mariadb-server"]` (Spec 2). (`Php`/`Mailpit`/`Composer` already empty.)
- `run_setup`: after the existing PHP/composer/mailpit blocks, install each missing component into the layout (best-effort, push errors to `report.errors`, record `config.versions`):
  - `Component::Nginx` missing → `nginx_static::install_nginx(paths, downloader, sink)` → record `versions["nginx"]`.
  - `Component::Redis` missing → `redis_static::install_redis(paths, downloader, runner, sink)` → record `versions["redis"]`.
  - `Component::Mkcert` missing → `mkcert_static::install_mkcert(paths, downloader, sink)` → record `versions["mkcert"]`.
  Add `nginx_fetched`/`redis_fetched`/`mkcert_fetched: bool` to `SetupReport` (mirror `mailpit_fetched`). Keep the config save + `apply_versions` at the end.
  - The mkcert CA step: `resolve_bin("mkcert", &managed_bin_dirs(paths))` then `install_mkcert_ca(&that_path)`.
  - `setcap nginx` already resolves via `managed_bin_dirs` (Spec 0 Task 4) — the downloaded nginx is found there.
  - `disable_system_services` / `allow_mariadb_apparmor` stay (best-effort; harmless when the distro units/profile are absent; mariadb still apt until Spec 2).
- `detect()`: change the `other` arm from `resolve_bin(&name, &[paths.bin()])` to `resolve_bin(&name, &crate::layout::managed_bin_dirs(paths))` (with `resolve_bin`'s `$PATH` fallback still covering apt mariadb until Spec 2, and the new layout binaries found under `bin/*/current`). This also fixes mailpit detection (now in `bin/mailpit/current`).

### 3.6 `lib.rs` re-exports
Add `pub mod mkcert_static; pub mod nginx_static; pub mod redis_static;` and re-export `install_mkcert`/`MkcertError`, `install_nginx`/`NginxError`, `install_redis`/`RedisError`.

## 4. Behavior & error handling
- **Self-contained:** after setup, `~/laralux/bin` holds `nginx/<ver>/nginx`, `redis/<ver>/{redis-server,redis-cli}`, `mkcert/<ver>/mkcert` alongside php/mailpit/coredns/composer — no distro packages for the stack except MariaDB (Spec 2).
- **Idempotent / best-effort:** installers skip when the versioned binary exists; a download failure for one tool records an error and doesn't abort the others; progress flows through `sink`.
- **No new root:** all three are user-space downloads; mkcert CA install is un-escalated (as today); the only privileged steps remain the existing ones (hosts, setcap, resolved drop-in).
- **Interim:** the `$PATH` fallback in `resolve_bin` means an externally-present nginx/redis still resolves if the download fails.

## 5. Testing (TDD; no network in unit tests)
- `mkcert_url`/`mkcert_arch`; `nginx_arch`/`latest_nginx` (JSON fixture with mixed arch/os/version → highest linux/<arch>; tolerant of malformed; `None` when no match); `valkey_url`/`valkey_arch` — exact strings + arch maps.
- `apt_packages_for`: assert `Nginx`/`Redis`/`Mkcert` are now empty and `Mariadb` still `["mariadb-server"]` (update existing tests).
- `Privileged::install_mkcert_ca`: `FakePrivileged` records the passed path.
- Installers (`install_*`) shell out / download and are **live-verified** (like `ensure_coredns`); the pure fns carry unit coverage. `run_setup` tests use the fakes; assert the new components attempt install + `config.versions` recorded (and apt no longer expected for nginx/redis/mkcert).
- `cargo test -p laralux-core`; `cargo build -p laralux-desktop && cargo build -p laraluxctl`.

## 6. Out of scope (Spec 2 / backlog)
- **MariaDB** tarball + datadir init (`mariadb-install-db`) — Spec 2; then remove `Privileged::apt_install`/`add_apt_repository`, the `disable_system_services` mariadb entry, and `allow_mariadb_apparmor`.
- Firefox/NSS trust beyond user-store `mkcert -install` (the `libnss3-tools`/`certutil` apt dep is dropped; document that Firefox trust may need `certutil` present, otherwise Chromium/system trust still works).
- Pinning/override UI for tool versions.
