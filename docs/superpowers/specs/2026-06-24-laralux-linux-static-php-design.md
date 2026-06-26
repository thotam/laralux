# Laralux Linux — Static PHP Binaries (Phase 2, Version slice 1b) Design Spec

**Date:** 2026-06-24
**Status:** Design approved, pending spec review
**Goal:** Source every PHP version from prebuilt **static** php-fpm binaries (dl.static-php.dev, `bulk` preset) downloaded into `~/laralux/bin`, replacing the apt/ondrej install path entirely. This makes PHP install/switch work on **any** distro (including Ubuntu 26.04 "resolute", where ondrej has no packages) and needs **no root/pkexec** (everything lives under the user-owned `~/laralux`).

This supersedes the apt/ondrej install added in the previous slice (PHP Version Management), which is removed here. Detection, the active-version switch (`Orchestrator::replace_php_version`), and the Settings UI from that slice are **kept unchanged** — they already work for any `php-fpm<minor>` found in `~/laralux/bin`.

---

## 1. Context & current state

PHP version is global (`Config.php_version`). `PhpFpmService` writes `etc/php/<ver>/php-fpm.conf`, listens on the **version-independent** socket `tmp/php-fpm.sock`, and spawns `php-fpm<version>`. Crucially, the orchestrator resolves a service's program name through `~/laralux/bin` before PATH (`orchestrator.rs:77`, via `bin::resolve_or_name`) — the same mechanism that runs the downloaded `mailpit` binary. So **a binary placed at `~/laralux/bin/php-fpm<minor>` is detected and spawned with no further wiring**:
- `bin::list_php_fpm_versions(&[paths.bin()])` already lists it,
- `php_versions(...)` marks it installed,
- `Orchestrator::replace_php_version(v)` spawns `php-fpm<v>` resolved from `~/laralux/bin`.

The previous slice installed PHP via `ppa:ondrej/php` + apt. On resolute the ondrej PPA 404s, and even pinned to the newest LTS suite (`noble`) the packages are uninstallable due to a system-library ABI gap (`libicu74`/`libxml2`/`libzip4t64` vs resolute's `libicu78`/`libxml2-16`/`libzip5`). Static binaries sidestep this: they are self-contained ELF executables with no system-library dependencies.

The Setup wizard currently apt-installs an unversioned distro PHP. This slice converts that to a static download too, so PHP comes from **one source only**.

### Verified facts (dl.static-php.dev, checked 2026-06-24)
- URL: `https://dl.static-php.dev/static-php-cli/bulk/php-<X.Y.Z>-fpm-linux-<arch>.tar.gz`.
- `bulk` fpm-linux-x86_64 minors available: **8.0, 8.1, 8.2, 8.3, 8.4, 8.5** (latest patches 8.0.30 / 8.1.34 / 8.2.31 / 8.3.31 / 8.4.22 / 8.5.7). **No 7.4** (EOL; not built statically).
- Architectures: `x86_64`, `aarch64`.
- The fpm tarball contains exactly one file, `php-fpm` (a statically-linked ELF; `php-fpm -v` → e.g. `PHP 8.4.22 (fpm-fcgi)`).
- The `bulk` build has **65 modules** including the full Laralux-parity set and more: `intl`, `mysqli`, `imagick`, `sodium`, `xsl`, `pdo_mysql`, `gd`, `redis`, `curl`, `mbstring`, `zip`, `exif`, `bcmath`, `gmp`, `soap`, `opcache`, `sqlite3`, `pdo_pgsql`, … (the `common` preset lacks `intl`/`mysqli`/`imagick`/`sodium`/`xsl`, so `bulk` is chosen).
- Directory JSON: `…/bulk/?format=json` returns an array of `{ name, size, last_modified, … }`, used to resolve a minor to its newest patch.

## 2. Approach (chosen: static-only, `bulk` preset, into `~/laralux/bin`)

Add a `php_static` core module that resolves the newest patch for a requested minor from the `bulk` directory JSON, downloads the fpm tarball, extracts the `php-fpm` binary, and installs it as `~/laralux/bin/php-fpm<minor>`. Wire it into both the version-manager Install command and the Setup wizard. Remove the apt/ondrej PHP path.

Rejected:
- **apt/ondrej** — broken on resolute (ABI gap), LTS-only; removed per decision.
- **`common` preset** — missing `intl`/`mysqli`/`imagick`/`sodium`/`xsl` (WordPress needs `mysqli`, Laravel needs `intl`).
- **`gnu-bulk`** — glibc-linked variant; the default `bulk` (musl, fully static, no libc dependency) is the safer "runs anywhere" choice.

## 3. Architecture & components

### 3.1 `core/src/php_static.rs` (new)

- `const STATIC_PHP_BASE: &str = "https://dl.static-php.dev/static-php-cli/bulk";`
- `pub fn arch_tag() -> Option<&'static str>` — maps `std::env::consts::ARCH`: `"x86_64" → "x86_64"`, `"aarch64" → "aarch64"`, else `None`.
- `pub fn latest_patch_url(version: &str, arch: &str, listing_json: &str) -> Option<String>` (pure, unit-tested):
  - parse `listing_json` (serde_json) as an array of objects with a `name` string;
  - keep names matching `php-<version>.<patch>-fpm-linux-<arch>.tar.gz` (exact minor + arch; `<patch>` an integer);
  - pick the **highest** `<patch>` numerically;
  - return `"<STATIC_PHP_BASE>/<name>"`, or `None` if the version/arch isn't present.
- `pub fn install_php_static(paths: &LaraluxPaths, version: &str, downloader: &dyn Downloader, runner: &dyn CommandRunner) -> Result<(), PhpStaticError>`:
  1. `arch = arch_tag()` (else `PhpStaticError::Arch`).
  2. fetch `…/bulk/?format=json` to `tmp/static-php-index.json` (Downloader); read it.
  3. `url = latest_patch_url(version, arch, &json)` (else `PhpStaticError::Unavailable(version)`).
  4. fetch `url` to `tmp/php-<version>-fpm.tar.gz` (Downloader).
  5. extract via `runner.run("tar", ["-xzf", <tarball>, "-C", <tmp>, "php-fpm"], None)` (CommandRunner).
  6. move/rename `tmp/php-fpm` → `~/laralux/bin/php-fpm<version>`; ensure mode `0o755` (`std::fs::set_permissions`). `bin/` is created if missing.
- `pub enum PhpStaticError` (thiserror): `Arch(String)`, `Unavailable(String)`, `Download(String)`, `Extract(String)`, `Io(std::io::Error)`.
- Seams reuse the existing `setup::Downloader` (real `CurlDownloader`, `FakeDownloader`) and `scaffold::CommandRunner` (real `RealCommandRunner`, `FakeCommandRunner`), so the orchestration is unit-tested without network/tools. Add `serde_json` to `core`'s dependencies (Cargo) for the listing parse.

### 3.2 `core/src/php_versions.rs` — catalog update, remove apt

- `KNOWN_PHP_VERSIONS` → **`["8.0", "8.1", "8.2", "8.3", "8.4", "8.5"]`** (drop `7.4`). Installed-but-unknown versions still appear via the existing union.
- **Remove** `apt_packages_for_php`, `ondrej_suite`, `ondrej_suite_for`, and the apt `install_php` (and its tests). `php_versions`/`php_versions_from`/`PhpVersionInfo` are unchanged. `PhpVersionError` is removed (replaced by `PhpStaticError`); the IPC layer maps `PhpStaticError`/"not installed" to strings.

### 3.3 `core/src/privileged.rs` — remove ondrej escalation

- **Remove** the `add_ondrej_php` trait method, the `ondrej_php_argv` helper, its `SudoPrivileged`/`PkexecPrivileged` impls, and the `FakePrivileged` `ondrej_suites` recorder + the `fail_apt` toggle/`set_fail_apt` (all added only for the apt path). `apt_install`/`add_apt_repository`/`disable_system_services` stay (still used by Setup for nginx/mariadb/redis/mailpit/composer).

### 3.4 `core/src/setup.rs` — Setup installs PHP statically

- `run_setup` gains a `runner: &dyn CommandRunner` parameter (so it can call `install_php_static`). Both callers (`commands.rs::run_setup_cmd`, `laraluxctl`) pass `&RealCommandRunner`.
- `Component::Php` stays in the checklist; `detect` still uses `detect_php_fpm_version(&[paths.bin()])` (now finds the static binary). **Remove** `Php` from `apt_packages_for`/`classify_apt` (PHP is no longer an apt package).
- In `run_setup`, when `Php` is missing, install the newest known version statically: `install_php_static(paths, DEFAULT_PHP_VERSION, downloader, runner)` where `DEFAULT_PHP_VERSION = "8.5"` (the newest in `KNOWN_PHP_VERSIONS`). On success, detect + persist `Config.php_version` (existing logic); on failure push to `report.errors`.
- `stack_units_to_disable` drops the `php<v>-fpm` unit (no distro php-fpm systemd unit exists anymore); it disables `nginx`/`mariadb`/`redis-server` only. `SetupReport.php_version` is retained.

### 3.5 IPC (Tauri) — `src-tauri/src/commands.rs`

- `install_php_version(app, version: String) -> Result<Vec<PhpVersionInfo>, String>` — **async + spawn_blocking**, now calls `install_php_static(&state.paths, &version, &CurlDownloader, &RealCommandRunner)` (no `Privileged`, no pkexec) → returns the refreshed catalog. Errors map to `Err(String)`.
- `php_versions()` and `set_php_version(...)` are **unchanged** (`set_php_version` still rejects a version not present in `list_php_fpm_versions`, persists `Config`, and calls `replace_php_version`).
- `run_setup_cmd` passes `&RealCommandRunner` to `run_setup`.

### 3.6 Frontend — `dist/` (minimal change)

- The Settings "PHP version" card is **unchanged in structure**. Only the Install affordance's busy copy changes to reflect a download (e.g. "Downloading…") instead of an apt run. Because `KNOWN_PHP_VERSIONS` no longer includes 7.4, the card lists 8.0–8.5; all are installable via static download, so every not-installed row shows **Install** and every installed row shows **Use**/**Active**.

## 4. Behavior details & decisions

- **No privilege for PHP**: static install writes only under `~/laralux` (`tmp`, `bin`) — no pkexec prompt for installing/switching PHP. (Setup still uses pkexec for the apt core stack + setcap + hosts.)
- **Version → patch resolution** is dynamic (reads the live `bulk` JSON), so new patch releases are picked up automatically; no hardcoded patch numbers.
- **Default Setup version** is `8.5` (newest known). Switching afterward installs other minors on demand.
- **7.4** is unavailable (EOL; no static build) and is not offered.
- **arch**: only `x86_64`/`aarch64`; other arches return `PhpStaticError::Arch` surfaced as a toast.
- The static `php-fpm` runs under the existing `PhpFpmService` invocation (`php-fpm<v> -F -y etc/php/<v>/php-fpm.conf`); the constant socket means nginx/vhosts are untouched on install or switch.

## 5. Error handling

- Typed `PhpStaticError` (arch/unavailable/download/extract/io) → `Err(String)` → toast; the Settings card stays usable.
- Setup PHP-install failure is non-fatal (collected into `SetupReport.errors`), consistent with the other setup steps.
- `set_php_version` on an uninstalled version → explicit `Err` (unchanged).

## 6. Testing (TDD; fakes only, no network/tools)

- `arch_tag`: returns a value on the test host (x86_64/aarch64) and the mapping is exercised; an internal pure mapping `arch_from(s)` is tested for `"x86_64"`, `"aarch64"`, and an unknown string → `None`.
- `latest_patch_url`: given a sample listing JSON containing `php-8.4.9-…`, `php-8.4.22-…`, `php-8.3.31-…` (x86_64) plus noise (cli/micro/aarch64 entries), returns the `8.4.22` x86_64 **bulk** URL for `("8.4","x86_64")`; returns `None` for a minor not present (`"7.4"`) and for a missing arch.
- `install_php_static` with module-local fakes: a downloader that writes a provided JSON for the index URL and dummy bytes for the tarball URL, and a CommandRunner that, on the `tar` call, creates a `php-fpm` file in the dest dir — assert (a) the index URL then the resolved tarball URL were fetched in order, (b) `tar -xzf … -C … php-fpm` was invoked, (c) `~/laralux/bin/php-fpm8.4` exists and is mode `0o755`; an unavailable version yields `PhpStaticError::Unavailable`.
- `setup`: `run_setup` (with `FakePrivileged` + `FakeDownloader` + a `FakeCommandRunner`) no longer puts any `php*` package in `report.apt_packages`; `classify_apt` excludes `Php`; `stack_units_to_disable` returns exactly `["nginx","mariadb","redis-server"]`.
- `php_versions`: `KNOWN_PHP_VERSIONS` is 8.0–8.5; catalog tests updated accordingly.

## 7. Out of scope (backlog)

- Node (nvm) and MariaDB version slices.
- Per-site PHP version.
- Choosing a non-`bulk` preset or a specific patch from the UI; uninstalling a version.
- Converting the rest of the stack (nginx/mariadb/redis/mailpit) to static binaries — only PHP changes here (mailpit is already a downloaded binary; the others stay apt).
- Windows/macOS arch handling.
