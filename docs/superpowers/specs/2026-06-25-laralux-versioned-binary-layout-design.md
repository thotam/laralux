# Laralux — Versioned Binary Layout (Spec 0, foundational) Design

**Date:** 2026-06-25
**Status:** Design approved, pending spec review
**Goal:** Restructure `~/laralux/bin` from a flat dir into a per-tool, per-version layout (`~/laralux/bin/<tool>/<version>/<binary>`) with a config-driven `current` symlink per tool, like Laralux for Windows. Config is the source of truth; symlinks are materialized from it. This is the **foundation** that Spec 1 (no-apt nginx/redis/mkcert) and Spec 2 (mariadb) install into.

---

## 1. Context & current state

Today every managed binary lives flat in `~/laralux/bin`:
- PHP: `php-fpm8.4`, `php8.4`, `php-fpm8.3`, `php8.3`, and a `php` symlink → the active `php<major.minor>` (see `php_static.rs`, `php_cli.rs`). Versions are tracked by **major.minor** (`install_php_static("8.4")` names binaries `php-fpm8.4`/`php8.4`, discarding the patch).
- mailpit, coredns, composer (+`composer.phar`) sit directly in `bin`.
- nginx, redis come from **apt** (in `/usr/sbin`, resolved via PATH) — converted to downloads in Spec 1.

`bin::resolve_bin(name, extra_dirs)` searches `extra_dirs` then `$PATH` then system dirs; callers pass `&[paths.bin()]`. PHP detection/listing scans for `php-fpm<maj>.<min>` filenames. `Config` holds `php_version` (major.minor), `tld`, `shell_integration`. Shell PATH integration (`shell_env.rs`) prepends `~/laralux/bin`. The composer wrapper runs `$(dirname)/php` + `$(dirname)/composer.phar`.

## 2. Approach (chosen: config-driven layout + per-tool `current` symlink)

- **Layout:** `~/laralux/bin/<tool>/<version>/<binary…>`. Each tool gets a `current` symlink: `~/laralux/bin/<tool>/current → <version>`.
- **Source of truth:** `Config.versions` (a `tool → version` map). The `current` symlinks are *materialized from config* on setup and on every version switch; config — not the filesystem — decides what is active.
- **Resolution:** binaries are found in `~/laralux/bin/*/current/`. `resolve_bin` already accepts `extra_dirs`, so we feed it the list of `current` dirs; the function itself is unchanged and its `$PATH` fallback still finds apt-installed tools during the Spec 0→1 interim.
- **No migration:** existing flat installs are not relocated. Detection keys off the new layout, so a re-run of setup downloads fresh into it (user decision: "bỏ cũ, setup tải lại sạch"). The old flat files are inert and may be deleted by the user.

**Rejected:** a Homebrew-style `opt/` + flat `bin/` symlink farm (user wants the version dirs *inside* `bin`); keeping the flat layout (user wants Laralux-style per-version dirs); migrating existing flat binaries in place (user chose a clean re-download).

## 3. Architecture & components

All changes are in `core` (zero Tauri deps). Versions are **full** (PHP `8.3.31`, not `8.3`); mailpit/composer versions are read from the downloaded binary (like PHP's real version), so every tool dir is a real version string.

### 3.1 Path/layout helpers — `core/src/layout.rs` (new) + `paths.rs`

- `LaraluxPaths::tool_dir(&self, tool: &str) -> PathBuf` → `bin/<tool>`.
- `LaraluxPaths::version_dir(&self, tool: &str, version: &str) -> PathBuf` → `bin/<tool>/<version>`.
- `LaraluxPaths::current_link(&self, tool: &str) -> PathBuf` → `bin/<tool>/current`.
- `core/src/layout.rs`:
  - `pub fn managed_bin_dirs(paths: &LaraluxPaths) -> Vec<PathBuf>`: read `bin/`; for each subdirectory `<tool>`, if `<tool>/current` exists, include `bin/<tool>/current`. (Order: sorted for determinism.) This is the `extra_dirs` every resolver call passes.
  - `pub fn set_current(paths: &LaraluxPaths, tool: &str, version: &str) -> std::io::Result<()>`: ensure `bin/<tool>` exists, remove any existing `current` (symlink/file), create symlink `current → <version>` (relative target = the bare version string, so it resolves inside the tool dir). Unix `symlink`.
  - `pub fn apply_versions(paths: &LaraluxPaths, config: &Config) -> Vec<String>`: for each `(tool, version)` in `config.versions`, if `bin/<tool>/<version>` exists, `set_current`; collect a warning string for any tool whose version dir is missing. Best-effort (never errors out the caller).
  - `pub fn installed_versions(paths: &LaraluxPaths, tool: &str) -> Vec<String>`: list subdirectories of `bin/<tool>` excluding `current`, returning the version strings (sorted, semver-ish numeric compare).

### 3.2 Config `versions` map — `core/src/config.rs`

- Add `pub versions: std::collections::BTreeMap<String, String>` (`#[serde(default)]`). Keys: `php`, `nginx`, `redis`, `mariadb`, `mailpit`, `coredns`, `mkcert`, `composer`.
- **Back-compat migration on load:** keep the existing `php_version` field (`#[serde(default)]`); after deserialization, if `versions` lacks a `php` entry but `php_version` is non-empty, set `versions["php"] = php_version`. (A `Config::normalize()` run inside `Config::load`.) New writes populate `versions`; `php_version` is retained for one cycle for older readers but is no longer authoritative.
- Helper: `Config::tool_version(&self, tool: &str) -> Option<&str>`.

### 3.3 Resolver wiring — `core/src/bin.rs` and callers

- `resolve_bin`/`resolve_or_name` stay as-is.
- Every current caller that passes `&[paths.bin()]` now passes `&managed_bin_dirs(paths)`:
  - `Orchestrator::do_start` — `resolve_or_name(&spec.program, &managed_bin_dirs(&self.paths))`.
  - `bin::ensure_nginx_bind_cap` — `resolve_bin("nginx", &managed_bin_dirs(paths))`.
  - `setup.rs` — `resolve_bin("nginx"/"mkcert", &managed_bin_dirs(paths))`.
  - `terminal.rs` `detect_terminal` uses `resolve_bin(_, &[])` (system terminals) — unchanged.
- **PHP detection/listing** moves from filename-scanning to dir-listing:
  - Replace `detect_php_fpm_version`/`list_php_fpm_versions` (which scan for `php-fpm<maj>.<min>` names) with `installed_versions(paths, "php")` (lists `bin/php/*/`). `detect` for `Component::Php` becomes "is `bin/php/current/php-fpm` resolvable" (i.e. config has an active php whose dir exists). Keep the old functions only if still referenced; otherwise remove them and their tests, replacing with `installed_versions` tests.

### 3.4 Installers write into the layout

Each installer places binaries under `bin/<tool>/<version>/` and calls `set_current`. The atomic temp→rename idiom is kept; only the destination dir changes.

- **PHP — `php_static.rs`:**
  - Refactor `latest_patch_url` → `latest_patch(version, arch, sapi, json) -> Option<(String /*full version e.g. "8.3.31"*/, String /*url*/)>` (it already parses the patch; now it returns it).
  - `install_php_static(paths, requested, downloader, runner) -> Result<String, PhpStaticError>` (returns the **full installed version**): resolve the full version from the fpm entry, create `bin/php/<full>/`, install `php-fpm` and `php` (bare names, no version suffix) into it, `set_current(paths, "php", &full)`. `requested` may be a major.minor (e.g. "8.4") or a full version; the index resolves the latest matching patch.
  - `install_php_cli` similarly installs `php` into `bin/php/<full>/` (used by the CLI-sync path).
- **mailpit — `setup.rs`:** download to `paths.tmp()`, read its version (`probe_version`, §3.5) → `ver`, move into `bin/mailpit/<ver>/mailpit`, `set_current(paths,"mailpit",&ver)`, record `versions["mailpit"]=ver`.
- **coredns — `coredns.rs` `ensure_coredns`:** install into `bin/coredns/<COREDNS_VERSION>/coredns`, `set_current(paths,"coredns",COREDNS_VERSION)`, record `versions["coredns"]`.
- **composer — `php_cli.rs` `install_composer`:** download `composer.phar` to `bin/composer/<ver>/composer.phar` where `ver` is read via `php composer.phar --version` (`probe_version`); write the `composer` wrapper next to it; `set_current(paths,"composer",&ver)`; record `versions["composer"]`. The wrapper now resolves php absolutely (§3.7).

### 3.5 Version probing — `core/src/layout.rs`

- `pub fn probe_version(program: &Path, args: &[&str]) -> Option<String>`: run `program args`, capture stdout+stderr, return the first `\d+\.\d+(\.\d+)?` token found. Used for mailpit (`["version"]`) and composer (run as `php composer.phar --version`). On failure (spawn error / no match) the caller falls back to a pinned const (`MAILPIT_FALLBACK_VERSION`, `COMPOSER_FALLBACK_VERSION`) so the install still lands in a named dir. PHP needs no probe (the index gives the exact version).

### 3.6 Consumer adaptations

- **`PhpFpmService` (`service/php_fpm.rs`):** `command` runs `php-fpm` (resolved via `bin/php/current/`) instead of `php-fpm<version>`. The service no longer embeds the version in the program name; the active version is whatever `bin/php/current` points to. (The pool/socket config is already version-independent.) `replace_php_version(version)` becomes: `set_current(paths,"php",version)` + restart php-fpm (no service struct swap needed, since the program name is constant `php-fpm`). Update `Orchestrator::replace_php_version` accordingly.
- **`php_cli.rs` `set_active_php(paths, version)`:** becomes `layout::set_current(paths, "php", version)` (drop the flat `php` symlink). `ensure_active_php_cli` checks `bin/php/<version>/php` existence.
- **`php_versions.rs`:** list from `installed_versions(paths, "php")`; the active version is `config.versions["php"]`.
- **`shell_env.rs`:** the rc block prepends `"$HOME/laralux/bin/php/current"` and `"$HOME/laralux/bin/composer/current"` (the two CLI tools a terminal needs), instead of `"$HOME/laralux/bin"`. Use literal `$HOME` (per-user, as today).
- **composer wrapper:** `#!/bin/sh\nexec "$HOME/laralux/bin/php/current/php" "$HOME/laralux/bin/composer/current/composer.phar" "$@"\n` (absolute via `$HOME`, since the wrapper and php now live in different tool dirs).

### 3.7 `setup.rs` `run_setup` — fresh install into the layout

- **Ordering:** install PHP before composer — `composer`'s version probe runs `bin/php/current/php composer.phar --version`, and its wrapper execs the active php, so php must be installed and `current`-linked first.
- After installing each component into the layout (§3.4), write the resolved versions into `config.versions` and persist the config, then `apply_versions(paths, &config)` to ensure all `current` symlinks exist.
- `detect(paths)` reports a component present when its active binary resolves under `managed_bin_dirs` (e.g. `resolve_bin("php-fpm", &managed_bin_dirs(paths)).is_some()` for PHP; `redis-server`, `nginx`, `mkcert`, `mailpit`, `coredns`, `composer` likewise). With no flat-layout fallback, an existing flat install reads as "absent" and is re-downloaded into the layout — the intended clean re-install.
- mkcert CA + `setcap nginx` resolve their binaries via `managed_bin_dirs` (nginx/mkcert still apt/absent until Spec 1, so PATH fallback covers nginx; mkcert is downloaded in Spec 1 — until then the CA step no-ops with a recorded note).

### 3.8 `lib.rs` re-exports

Re-export `layout::{managed_bin_dirs, set_current, apply_versions, installed_versions, probe_version}`. Keep existing re-exports; adjust signatures changed above (`install_php_static` now returns `String`).

## 4. Behavior, currency & error handling

- **Config-authoritative:** `apply_versions` reconciles symlinks from `config.versions` at startup/setup/switch; the filesystem never silently overrides config.
- **Interim correctness:** until Spec 1, nginx/redis come from apt and resolve via the `$PATH` fallback in `resolve_bin`; the layout change does not break them.
- **Idempotent:** installers skip when `bin/<tool>/<version>/<binary>` already exists; `set_current` is a cheap symlink repoint.
- **Best-effort:** `apply_versions` and per-component installs collect warnings/errors without aborting the rest (matches today's `run_setup`).
- **Switching:** changing a tool's version = write `config.versions[tool]` + `set_current` + restart that service. Multiple patches of the same minor can coexist (`bin/php/8.3.31`, `bin/php/8.3.35`).

## 5. Testing (TDD; no network in unit tests)

- `layout::set_current` / `managed_bin_dirs` / `installed_versions`: build a temp `bin/` with `nginx/1.31.2/nginx`, `php/8.3.31/php-fpm`, set currents, assert `managed_bin_dirs` returns the `current` dirs and `resolve_bin("nginx", &dirs)` finds the binary; `installed_versions` lists versions excluding `current`; re-`set_current` repoints.
- `Config` migration: a TOML with `php_version="8.3"` and no `versions` → after load, `versions["php"]=="8.3"`.
- `apply_versions`: config with a version whose dir is missing → returns a warning, creates the others.
- `probe_version`: feed a fake program (a shell script printing `v1.2.3` / `Composer version 2.7.0 …`) → returns `"1.2.3"` / `"2.7.0"`; non-matching output → `None`.
- `php_static::latest_patch`: reuse the existing `SAMPLE` index → returns `("8.4.22", url)` for `("8.4", x86_64, fpm)`.
- Updated installer tests assert binaries land under `bin/<tool>/<version>/` and `current` points at the version (replacing the old flat-path assertions, e.g. `install_php_static` now checks `bin/php/8.4.22/php-fpm` + `bin/php/current → 8.4.22` and the returned version string).
- `set_active_php`/`replace_php_version`: assert `bin/php/current` repoints and (for replace) php-fpm restarts.
- `run_setup` tests (FakePrivileged/FakeDownloader/FakeCommandRunner): assert components install into the layout and `config.versions` is populated.

## 6. Out of scope (later specs / backlog)

- **Spec 1:** no-apt static **nginx / redis(Valkey) / mkcert** installers — they install into this layout from the start, and drop the apt path. (The already-written Spec-1 design is updated to target `bin/<tool>/<version>/`.)
- **Spec 2:** **mariadb** static tarball + datadir init.
- A Settings UI to pick/pin versions per tool, and an "installed versions" management view.
- Garbage-collecting old version dirs (keep-N policy).
- Windows-style behavior beyond Linux symlinks (the design is config-first, so a future non-symlink platform can build paths straight from `config.versions`).
