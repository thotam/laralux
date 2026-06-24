# Laragon Linux — PHP Version Management (Phase 2, Version slice 1) Design Spec

**Date:** 2026-06-24
**Status:** Design approved, pending spec review
**Goal:** Install additional PHP versions and switch the stack's active PHP version from the GUI. The active version drives the orchestrated `php-fpm`; switching restarts php-fpm on the new version without touching nginx/vhosts.

This is the **first slice** of the Phase-2 "Cài/đổi version" item. The other two subsystems are separate, later slices and out of scope here:
- **Node via nvm** (not an orchestrated service; a different model).
- **MariaDB version** (heavy: datadir/migration risk).

Per-site PHP version is also out of scope (this slice manages one global active version).

---

## 1. Context & current state

PHP version is **global today**: `Config.php_version` is a single string (default `"8.4"`). `service::registry::build_services` constructs `PhpFpmService::new(config.php_version)`. The php-fpm service:
- writes its pool config to `etc/php/<version>/php-fpm.conf` (`PhpFpmService::conf_path`),
- listens on a **version-independent** socket `tmp/php-fpm.sock` (`PhpFpmService::socket_path`),
- spawns the binary `php-fpm<version>` (e.g. `php-fpm8.4`).

Because the socket path is constant, every site vhost's `fastcgi_pass unix:.../php-fpm.sock` is unaffected by a version change — **switching PHP requires no vhost regeneration and no nginx reload**.

`core::bin` already has `detect_php_fpm_version(extra_dirs) -> Option<String>` (finds one installed version) and `parse_php_version`. The setup wizard installs unversioned distro PHP and persists the detected version into `Config`. The `Privileged` trait already exposes `add_apt_repository`, `apt_install`, and `disable_system_services`.

The `Orchestrator` holds `services: Vec<Box<dyn Service>>` and owns running process handles, so the active php-fpm version can be swapped at runtime by replacing the php-fpm entry.

## 2. Approach (chosen: manage installed versions + global active switch)

Detect installed `php-fpm<X.Y>` binaries, present a fixed known list (7.4–8.5) annotated with installed/active state, install a chosen version via `ppa:ondrej/php` + apt, and switch the active version by persisting `Config.php_version` and swapping the orchestrator's php-fpm service (restarting it if it was running). Install via the ondrej PPA is the standard multi-version source; where the PPA is unavailable (e.g. a non-LTS Ubuntu like resolute that 404s), install surfaces a clear error while **switching among already-installed versions still works**.

Rejected:
- **Per-site PHP version** — larger model change (vhost-level FPM routing); deferred.
- **Switch-only, no install** — the user explicitly wants install + switch.
- **Bundled/static PHP builds** — heavy; off-pattern for the native-services design.

## 3. Architecture & components

### 3.1 `core/src/bin.rs` — list installed versions

- `pub fn list_php_fpm_versions(extra_dirs: &[PathBuf]) -> Vec<String>` — scan `extra_dirs` + `PATH` dirs + the standard sbin dirs already used by `detect_php_fpm_version` for executables named `php-fpm<maj>.<min>`; parse with the existing `parse_php_version`; return **sorted, de-duplicated** `"<maj>.<min>"` strings (ascending). Empty when none found. (`detect_php_fpm_version` stays as-is for the setup wizard's single-version detection.)

### 3.2 `core/src/php_versions.rs` (new) — version catalog + install

- `pub const KNOWN_PHP_VERSIONS: [&str; 7] = ["7.4", "8.0", "8.1", "8.2", "8.3", "8.4", "8.5"];`
- `pub struct PhpVersionInfo { pub version: String, pub installed: bool, pub active: bool }` (serde `Serialize`).
- `pub fn php_versions(paths: &LaragonPaths, active: &str) -> Vec<PhpVersionInfo>`:
  - `installed = list_php_fpm_versions(&[paths.bin()])`.
  - Start from `KNOWN_PHP_VERSIONS`; **union** in any installed version not in the known list (so a manually-installed odd version still shows). Sort ascending by `(maj, min)`.
  - For each: `installed = installed_set.contains(v)`, `active = (v == active)`.
- `pub fn apt_packages_for_php(version: &str) -> Vec<String>` → the **Laragon-parity** baseline, version-pinned (in this order):
  `["php<v>-fpm","php<v>-cli","php<v>-curl","php<v>-gd","php<v>-intl","php<v>-imagick","php<v>-mbstring","php<v>-mysql","php<v>-sqlite3","php<v>-xml","php<v>-xsl","php<v>-zip","php<v>-redis"]`.
  This mirrors the extensions Laragon enables by default (curl, fileinfo, gd, intl, imagick, mbstring, exif, mysqli, openssl, pdo_mysql, pdo_sqlite, redis, sodium, xsl, zip): `php<v>-mysql` provides mysqli+pdo_mysql, `php<v>-sqlite3` provides pdo_sqlite, `php<v>-xml` provides dom/xml. `openssl`, `fileinfo`, `exif`, and `sodium` are bundled in `php<v>-common` (pulled in transitively by `-cli`/`-fpm`), so they need no separate package.
- `pub fn install_php(version: &str, privileged: &dyn Privileged) -> Result<(), PhpVersionError>`:
  1. `privileged.add_apt_repository("ppa:ondrej/php")` — on failure return `PhpVersionError::Repo`.
  2. `privileged.apt_install(&apt_packages_for_php(version))` — on failure return `PhpVersionError::Apt` (this is where a 404/unavailable PPA surfaces).
  3. `privileged.disable_system_services(&[format!("php{version}-fpm")])` — best-effort; a failure is **not** fatal (the app runs its own php-fpm; the distro unit is only disabled to avoid a stray service). Errors here are swallowed (logged by the caller if desired), matching the "disable is non-fatal" rule from the setup wizard.
- `pub enum PhpVersionError` (thiserror): `Repo(String)`, `Apt(String)`, `NotInstalled(String)`.

(`add_apt_repository`/`apt_install`/`disable_system_services` signatures are the existing `Privileged` methods; `install_php` takes `&dyn Privileged` so it is unit-tested with `FakePrivileged`.)

### 3.3 `core/src/orchestrator.rs` — runtime version swap

- `pub fn replace_php_version(&mut self, version: &str) -> Result<bool, ServiceError>`:
  1. `was_running = self.state(ServiceKind::PhpFpm) == ServiceState::Running`.
  2. If running, `self.stop(ServiceKind::PhpFpm)`.
  3. Remove the existing php-fpm entry from `services` and push `Box::new(PhpFpmService::new(version))`.
  4. If `was_running`, `self.start(ServiceKind::PhpFpm)?` (start writes the new version's config and spawns `php-fpm<version>`).
  5. Return `was_running`.
- The socket is unchanged, so nginx keeps serving; no nginx interaction is needed here.

### 3.4 IPC (Tauri) — `src-tauri/src/commands.rs`

- `php_versions() -> Result<Vec<PhpVersionInfo>, String>` (sync): load `Config`, return `php_versions(&paths, &config.php_version)`.
- `install_php_version(app, version: String) -> Result<Vec<PhpVersionInfo>, String>` (async + `spawn_blocking`): `install_php(&version, &PkexecPrivileged)` → on success reload `Config` and return the refreshed `php_versions(...)`. Errors map to `Err(String)` for a toast.
- `set_php_version(app, version: String) -> Result<Vec<ServiceStatus>, String>` (async + `spawn_blocking`):
  1. Reject if `version` is not installed (`list_php_fpm_versions` doesn't contain it) → `Err("PHP <version> is not installed")`.
  2. Load `Config`, set `config.php_version = version`, `config.save(&paths.config_file())`.
  3. Lock the orchestrator, `orch.replace_php_version(&version)` (restarts php-fpm if it was running), return `orch.snapshot()`. The brief slow work (php-fpm restart) is off the main thread via `spawn_blocking`.
- Register the three commands in `main.rs` `generate_handler!`.

### 3.5 Frontend — Settings (`dist/`)

- Add a **"PHP version"** card to the Settings view. On entering Settings (or app load), call `php_versions` and store in `state.phpVersions`.
- Render the active version prominently, then a row per `PhpVersionInfo` (the 7 known + any extra), each showing its state and an action:
  - **active** → a non-interactive "Active" badge.
  - **installed & not active** → a **"Use"** button → `invoke("set_php_version", { version })`, spinner, success toast ("PHP <v> is now active"), refresh status + versions.
  - **not installed** → an **"Install"** button → `invoke("install_php_version", { version })`, spinner + "Installing… (apt, can take a few minutes)", success toast, refresh versions. Error toast keeps the row actionable.
- No `alert()`; reuse the toast system and existing button/card styles. Disable buttons while a PHP action is busy.

## 4. Behavior details & decisions

- **Active vs running:** `set_php_version` only changes the active version and restarts php-fpm **if it was already running**. If the stack is stopped, it just records the new active version (next "Start All" uses it). The UI does not force-start php-fpm on switch.
- **Switching to a non-installed version is rejected** (the UI only offers "Use" for installed versions, and the command double-checks).
- **Install does not auto-switch.** After installing, the user explicitly clicks "Use" to activate it (clear, predictable). The list refresh shows the new version as installed.
- **PPA unavailable (resolute):** `install_php` returns `PhpVersionError::Apt(...)` with the apt error → error toast; the rest of the feature (switching installed versions) is unaffected.
- **Extension baseline** on install: the Laragon-parity set (fpm, cli, curl, gd, intl, imagick, mbstring, mysql, sqlite3, xml, xsl, zip, redis — see §3.2), which is a superset of the setup wizard's unversioned baseline. `php<v>-imagick` comes from the ondrej PPA; if a particular package is unavailable for a version, apt fails the whole install and the error is surfaced (no partial cherry-picking in this slice).

## 5. Error handling

- Typed `PhpVersionError` → `Err(String)` → toast; the Settings card stays usable.
- `set_php_version` on an uninstalled version → explicit `Err`.
- `replace_php_version` start failure → propagated as `ServiceError` → `Err(String)`; php-fpm is left Stopped (start already reverts state on failure), so the UI shows the real state on next refresh.
- `disable_system_services` failure during install is non-fatal.

## 6. Testing (TDD; fakes only, no apt/network)

- `bin::list_php_fpm_versions`: a temp dir with `php-fpm8.3` + `php-fpm8.4` (and a non-matching file) → `["8.3","8.4"]` sorted; empty dir → `[]`.
- `php_versions`: with installed `{"8.4"}` and active `"8.4"` → every known version present; `8.4` is `installed && active`; `8.3` is `!installed`; an installed-but-unknown version (e.g. `"8.2"` present) is included; list sorted ascending.
- `apt_packages_for_php("8.3")` == the 13 Laragon-parity `php8.3-*` packages in the specified order (asserts `php8.3-fpm` first and that `php8.3-gd`, `php8.3-imagick`, `php8.3-redis`, `php8.3-xsl`, `php8.3-zip`, `php8.3-sqlite3` are present).
- `install_php("8.3", &FakePrivileged)`: records one `add_apt_repository("ppa:ondrej/php")`, one `apt_install` with the 13 `php8.3-*` packages, and one `disable_system_services(["php8.3-fpm"])`; a `FakePrivileged` configured to fail apt yields `PhpVersionError::Apt`.
- `Orchestrator::replace_php_version` (FakeSpawner): when php-fpm is running, returns `Ok(true)` and the service is running afterward on the new version; when stopped, returns `Ok(false)` and does not start it; the swapped service spawns `php-fpm<newversion>` (assert via the fake spawner's recorded program).

## 7. Out of scope (backlog)

- Per-site PHP version (vhost-level FPM routing).
- Node (nvm) and MariaDB version slices.
- Uninstalling a PHP version from the GUI.
- Choosing the extension set / installing extra extensions per version.
- PHP quick-settings (xdebug, memory_limit, …) — a separate Phase-2 item.
