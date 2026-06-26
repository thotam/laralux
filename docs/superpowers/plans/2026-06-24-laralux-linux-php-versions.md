# PHP Version Management (Phase 2, Version slice 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Install additional PHP versions (ondrej PPA, Laralux-parity extensions) and switch the stack's active PHP version from the GUI Settings, restarting php-fpm on the new version without touching nginx/vhosts.

**Architecture:** Detect installed `php-fpm<X.Y>` binaries, present a fixed known list (7.4–8.5) annotated installed/active, install a version via `ppa:ondrej/php` + apt, and switch the active version by persisting `Config.php_version` and swapping the orchestrator's php-fpm service (restarting it if running). The php-fpm socket is version-independent, so no vhost/nginx changes are needed.

**Tech Stack:** Rust (laralux-core, zero Tauri deps), Tauri 2, vanilla JS frontend (`dist/`, `withGlobalTauri`).

## Global Constraints

- `core` keeps **zero Tauri deps**.
- Commit messages MUST NOT contain a `Co-Authored-By` trailer.
- TDD: failing test first, watch it fail, implement, watch it pass, commit.
- Known versions: `["7.4","8.0","8.1","8.2","8.3","8.4","8.5"]` (a manually-installed version outside this list still appears).
- Install baseline (Laralux parity), version-pinned, in this order:
  `php<v>-fpm php<v>-cli php<v>-curl php<v>-gd php<v>-intl php<v>-imagick php<v>-mbstring php<v>-mysql php<v>-sqlite3 php<v>-xml php<v>-xsl php<v>-zip php<v>-redis`.
- Install source: `add-apt-repository ppa:ondrej/php` then apt; `disable_system_services(["php<v>-fpm"])` afterwards is **best-effort** (non-fatal).
- Switching to a non-installed version is rejected.
- Switching restarts php-fpm only if it was already running; the socket is constant so nginx/vhosts are untouched.
- Run core tests with `cargo test -p laralux-core`; build with `cargo build -p laralux-desktop`. If `cargo`/`node` are not on PATH use `$HOME/.cargo/bin/cargo` and `$HOME/.nvm/versions/node/v24.16.0/bin/node`.

---

### Task 1: `bin::list_php_fpm_versions` — list installed versions

**Files:**
- Modify: `core/src/bin.rs`
- Modify: `core/src/lib.rs` (re-export `list_php_fpm_versions`)

**Interfaces:**
- Consumes: existing `parse_php_version`, `FALLBACK_DIRS`.
- Produces:
  - `list_php_fpm_versions_in(dirs: &[PathBuf]) -> Vec<String>` (private; scans exactly `dirs`).
  - `pub fn list_php_fpm_versions(extra_dirs: &[PathBuf]) -> Vec<String>` (adds PATH + fallback, returns sorted unique `"<maj>.<min>"`).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `core/src/bin.rs`:

```rust
    #[test]
    fn lists_all_php_fpm_versions_sorted() {
        let dir = std::env::temp_dir().join(format!("lara-phplist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("php-fpm8.4"), "x").unwrap();
        std::fs::write(dir.join("php-fpm8.3"), "x").unwrap();
        std::fs::write(dir.join("php-fpm"), "x").unwrap();      // no version → ignored
        std::fs::write(dir.join("nginx"), "x").unwrap();        // unrelated → ignored
        let got = list_php_fpm_versions_in(&[dir.clone()]);
        assert_eq!(got, vec!["8.3".to_string(), "8.4".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lists_empty_when_none() {
        let dir = std::env::temp_dir().join(format!("lara-phplist-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(list_php_fpm_versions_in(&[dir.clone()]).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p laralux-core bin`
Expected: FAIL to compile — `list_php_fpm_versions_in` not found.

- [ ] **Step 3: Implement the functions**

In `core/src/bin.rs`, add after `detect_php_fpm_version`:

```rust
/// Scan exactly `dirs` for all `php-fpm<maj>.<min>` binaries; sorted unique versions.
fn list_php_fpm_versions_in(dirs: &[PathBuf]) -> Vec<String> {
    let mut found: Vec<(u32, u32)> = Vec::new();
    for dir in dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if !entry.path().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(ver) = name.strip_prefix("php-fpm") {
                if let Some((maj, min)) = parse_php_version(ver) {
                    if !found.contains(&(maj, min)) {
                        found.push((maj, min));
                    }
                }
            }
        }
    }
    found.sort();
    found.into_iter().map(|(maj, min)| format!("{maj}.{min}")).collect()
}

/// All installed php-fpm versions across `extra_dirs` + PATH + system bin dirs.
pub fn list_php_fpm_versions(extra_dirs: &[PathBuf]) -> Vec<String> {
    let mut dirs: Vec<PathBuf> = extra_dirs.to_vec();
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.extend(FALLBACK_DIRS.iter().map(PathBuf::from));
    list_php_fpm_versions_in(&dirs)
}
```

- [ ] **Step 4: Re-export in lib.rs**

In `core/src/lib.rs`, add (near the other re-exports):

```rust
pub use bin::list_php_fpm_versions;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p laralux-core bin`
Expected: PASS — the 2 new tests plus existing bin tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/bin.rs core/src/lib.rs
git commit -m "feat(core): list all installed php-fpm versions"
```

---

### Task 2: `php_versions` module — catalog + install

**Files:**
- Create: `core/src/php_versions.rs`
- Modify: `core/src/privileged.rs` (add a `fail_apt` toggle to `FakePrivileged`)
- Modify: `core/src/lib.rs` (declare module + re-exports)

**Interfaces:**
- Consumes: `list_php_fpm_versions`, `LaraluxPaths`, `Privileged` (`add_apt_repository`/`apt_install`/`disable_system_services`).
- Produces:
  - `KNOWN_PHP_VERSIONS: [&str; 7]`.
  - `struct PhpVersionInfo { version: String, installed: bool, active: bool }` (serde Serialize).
  - `php_versions_from(installed: &[String], active: &str) -> Vec<PhpVersionInfo>` (pure).
  - `php_versions(paths: &LaraluxPaths, active: &str) -> Vec<PhpVersionInfo>` (wrapper).
  - `apt_packages_for_php(version: &str) -> Vec<String>`.
  - `install_php(version: &str, privileged: &dyn Privileged) -> Result<(), PhpVersionError>`.
  - `enum PhpVersionError { Repo, Apt, NotInstalled }`.
  - `FakePrivileged::set_fail_apt(bool)` for the failure test.

- [ ] **Step 1: Add a failure toggle to `FakePrivileged`**

In `core/src/privileged.rs`, add a field to `struct FakePrivileged`:

```rust
    fail_apt: Arc<Mutex<bool>>,
```

Add an accessor in `impl FakePrivileged`:

```rust
    pub fn set_fail_apt(&self, fail: bool) {
        *self.fail_apt.lock().unwrap() = fail;
    }
```

Change `FakePrivileged::apt_install` to honor it:

```rust
    fn apt_install(&self, packages: &[String]) -> Result<(), PrivError> {
        if *self.fail_apt.lock().unwrap() {
            return Err(PrivError::Command("apt failed (test)".to_string()));
        }
        self.apt_installs.lock().unwrap().push(packages.to_vec());
        Ok(())
    }
```

(`PrivError::Command(String)` is the existing string-carrying variant in `privileged.rs`.)

- [ ] **Step 2: Write the failing module tests**

Create `core/src/php_versions.rs` with the test module first (rest compiles after Step 4):

```rust
use crate::bin::list_php_fpm_versions;
use crate::paths::LaraluxPaths;
use crate::privileged::Privileged;
use serde::Serialize;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privileged::FakePrivileged;

    #[test]
    fn php_versions_marks_installed_and_active() {
        let infos = php_versions_from(&["8.2".to_string(), "8.4".to_string()], "8.4");
        // every known version present
        for v in KNOWN_PHP_VERSIONS {
            assert!(infos.iter().any(|i| i.version == v), "missing {v}");
        }
        let by = |v: &str| infos.iter().find(|i| i.version == v).unwrap().clone();
        assert!(by("8.4").installed && by("8.4").active);
        assert!(by("8.2").installed && !by("8.2").active);
        assert!(!by("8.3").installed && !by("8.3").active);
        // sorted ascending
        let vers: Vec<String> = infos.iter().map(|i| i.version.clone()).collect();
        let mut sorted = vers.clone();
        sorted.sort_by_key(|v| {
            let mut it = v.split('.');
            (it.next().unwrap().parse::<u32>().unwrap(), it.next().unwrap().parse::<u32>().unwrap())
        });
        assert_eq!(vers, sorted);
    }

    #[test]
    fn php_versions_includes_unknown_installed() {
        let infos = php_versions_from(&["8.9".to_string()], "8.4");
        assert!(infos.iter().any(|i| i.version == "8.9" && i.installed));
    }

    #[test]
    fn apt_packages_are_laralux_parity() {
        let pkgs = apt_packages_for_php("8.3");
        assert_eq!(pkgs.len(), 13);
        assert_eq!(pkgs[0], "php8.3-fpm");
        for ext in ["php8.3-gd", "php8.3-imagick", "php8.3-redis", "php8.3-xsl", "php8.3-zip", "php8.3-sqlite3", "php8.3-mysql"] {
            assert!(pkgs.contains(&ext.to_string()), "missing {ext}");
        }
    }

    #[test]
    fn install_php_adds_ppa_installs_and_disables_unit() {
        let p = FakePrivileged::new();
        let repos = p.add_repos();
        let installs = p.apt_installs();
        let disabled = p.disabled_services();
        install_php("8.3", &p).unwrap();
        assert_eq!(repos.lock().unwrap().as_slice(), &["ppa:ondrej/php".to_string()]);
        assert_eq!(installs.lock().unwrap()[0], apt_packages_for_php("8.3"));
        assert_eq!(disabled.lock().unwrap()[0], vec!["php8.3-fpm".to_string()]);
    }

    #[test]
    fn install_php_surfaces_apt_error() {
        let p = FakePrivileged::new();
        p.set_fail_apt(true);
        assert!(matches!(install_php("8.3", &p), Err(PhpVersionError::Apt(_))));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p laralux-core php_versions`
Expected: FAIL to compile — module items not yet defined.

- [ ] **Step 4: Implement the module**

Add above the `#[cfg(test)]` block in `core/src/php_versions.rs`:

```rust
pub const KNOWN_PHP_VERSIONS: [&str; 7] = ["7.4", "8.0", "8.1", "8.2", "8.3", "8.4", "8.5"];

#[derive(Debug, thiserror::Error)]
pub enum PhpVersionError {
    #[error("add ondrej PPA failed: {0}")]
    Repo(String),
    #[error("apt install failed: {0}")]
    Apt(String),
    #[error("php {0} is not installed")]
    NotInstalled(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhpVersionInfo {
    pub version: String,
    pub installed: bool,
    pub active: bool,
}

fn vkey(v: &str) -> (u32, u32) {
    let mut it = v.split('.');
    let maj = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let min = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (maj, min)
}

/// Build the version catalog from a known list unioned with installed versions.
pub fn php_versions_from(installed: &[String], active: &str) -> Vec<PhpVersionInfo> {
    let mut versions: Vec<String> = KNOWN_PHP_VERSIONS.iter().map(|s| s.to_string()).collect();
    for v in installed {
        if !versions.contains(v) {
            versions.push(v.clone());
        }
    }
    versions.sort_by_key(|v| vkey(v));
    versions
        .into_iter()
        .map(|v| PhpVersionInfo {
            installed: installed.contains(&v),
            active: v == active,
            version: v,
        })
        .collect()
}

/// Version catalog using the live filesystem (PATH + ~/laralux/bin + system dirs).
pub fn php_versions(paths: &LaraluxPaths, active: &str) -> Vec<PhpVersionInfo> {
    php_versions_from(&list_php_fpm_versions(&[paths.bin()]), active)
}

/// The Laralux-parity, version-pinned apt package set for a PHP version.
pub fn apt_packages_for_php(version: &str) -> Vec<String> {
    [
        "fpm", "cli", "curl", "gd", "intl", "imagick", "mbstring", "mysql", "sqlite3", "xml",
        "xsl", "zip", "redis",
    ]
    .iter()
    .map(|ext| format!("php{version}-{ext}"))
    .collect()
}

/// Install a PHP version via the ondrej PPA, then disable its distro fpm unit.
pub fn install_php(version: &str, privileged: &dyn Privileged) -> Result<(), PhpVersionError> {
    privileged
        .add_apt_repository("ppa:ondrej/php")
        .map_err(|e| PhpVersionError::Repo(e.to_string()))?;
    privileged
        .apt_install(&apt_packages_for_php(version))
        .map_err(|e| PhpVersionError::Apt(e.to_string()))?;
    // Best-effort: keep the app in charge of php-fpm; the distro unit is just noise.
    let _ = privileged.disable_system_services(&[format!("php{version}-fpm")]);
    Ok(())
}
```

- [ ] **Step 5: Declare the module and re-export**

In `core/src/lib.rs`, add `pub mod php_versions;` (near `pub mod setup;`) and:

```rust
pub use php_versions::{install_php, php_versions, PhpVersionError, PhpVersionInfo};
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p laralux-core php_versions privileged`
Expected: PASS — the 5 new module tests and the existing privileged tests (the `fail_apt` default is `false`, so existing behavior is unchanged).

- [ ] **Step 7: Commit**

```bash
git add core/src/php_versions.rs core/src/privileged.rs core/src/lib.rs
git commit -m "feat(core): php version catalog + install (ondrej, Laralux-parity exts)"
```

---

### Task 3: `Orchestrator::replace_php_version`

**Files:**
- Modify: `core/src/orchestrator.rs`

**Interfaces:**
- Consumes: `ServiceKind`, `ServiceState`, existing `start`/`stop`/`state`, `PhpFpmService`.
- Produces: `pub fn replace_php_version(&mut self, version: &str) -> Result<bool, ServiceError>` (returns whether php-fpm had been running).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `core/src/orchestrator.rs`:

```rust
    #[test]
    fn replace_php_version_restarts_when_running() {
        let tmp = std::env::temp_dir().join(format!("lara-orch-php-{}", std::process::id()));
        let paths = LaraluxPaths::new(tmp.clone());
        let spawner = crate::process::FakeSpawner::new();
        let log = spawner.log();
        let mut orch = Orchestrator::new(
            paths,
            vec![Box::new(crate::service::php_fpm::PhpFpmService::new("8.4"))],
            Box::new(spawner),
        );
        orch.start(ServiceKind::PhpFpm).unwrap();
        assert_eq!(orch.state(ServiceKind::PhpFpm), ServiceState::Running);

        let was_running = orch.replace_php_version("8.3").unwrap();
        assert!(was_running);
        assert_eq!(orch.state(ServiceKind::PhpFpm), ServiceState::Running);
        // the most recent spawn used the new version's binary
        let progs: Vec<String> = log.lock().unwrap().iter().map(|s| s.program.clone()).collect();
        assert_eq!(progs.last().unwrap(), "php-fpm8.3");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn replace_php_version_does_not_start_when_stopped() {
        let tmp = std::env::temp_dir().join(format!("lara-orch-php2-{}", std::process::id()));
        let paths = LaraluxPaths::new(tmp.clone());
        let spawner = crate::process::FakeSpawner::new();
        let log = spawner.log();
        let mut orch = Orchestrator::new(
            paths,
            vec![Box::new(crate::service::php_fpm::PhpFpmService::new("8.4"))],
            Box::new(spawner),
        );
        let was_running = orch.replace_php_version("8.3").unwrap();
        assert!(!was_running);
        assert_eq!(orch.state(ServiceKind::PhpFpm), ServiceState::Stopped);
        assert!(log.lock().unwrap().is_empty()); // nothing spawned
        std::fs::remove_dir_all(&tmp).ok();
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p laralux-core orchestrator`
Expected: FAIL to compile — `replace_php_version` not found.

- [ ] **Step 3: Implement the method**

In `core/src/orchestrator.rs`, add to `impl Orchestrator` (after `stop`):

```rust
    /// Swap the active php-fpm version. Stops php-fpm if running, replaces the
    /// service with the new version, and restarts it iff it had been running.
    /// Returns whether php-fpm had been running. The socket is version-independent,
    /// so nginx/vhosts are unaffected.
    pub fn replace_php_version(&mut self, version: &str) -> Result<bool, ServiceError> {
        let was_running = self.state(ServiceKind::PhpFpm) == ServiceState::Running;
        if was_running {
            let _ = self.stop(ServiceKind::PhpFpm);
        }
        self.services.retain(|s| s.kind() != ServiceKind::PhpFpm);
        self.services
            .push(Box::new(crate::service::php_fpm::PhpFpmService::new(version)));
        if was_running {
            self.start(ServiceKind::PhpFpm)?;
        }
        Ok(was_running)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p laralux-core orchestrator`
Expected: PASS — both new tests plus existing orchestrator tests.

- [ ] **Step 5: Commit**

```bash
git add core/src/orchestrator.rs
git commit -m "feat(core): Orchestrator::replace_php_version (runtime php-fpm swap)"
```

---

### Task 4: IPC commands — `php_versions`, `install_php_version`, `set_php_version`

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs`

**Interfaces:**
- Consumes: `php_versions` (core wrapper), `install_php`, `list_php_fpm_versions`, `PhpVersionError`, `PhpVersionInfo`, `Orchestrator::replace_php_version`, `Config`.
- Produces:
  - `php_versions() -> Result<Vec<PhpVersionInfo>, String>` (sync).
  - `install_php_version(app, version: String) -> Result<Vec<PhpVersionInfo>, String>` (async).
  - `set_php_version(app, version: String) -> Result<Vec<ServiceStatus>, String>` (async).

- [ ] **Step 1: Extend the core imports**

In `src-tauri/src/commands.rs`, add to the first `use laralux_core::{...}` block (aliasing the catalog fn so it doesn't clash with the command named `php_versions`):

```rust
use laralux_core::{
    install_php, list_php_fpm_versions, php_versions as core_php_versions, PhpVersionError,
    PhpVersionInfo,
};
```

(Place this as a second `use laralux_core::{...}` line beneath the existing imports — do not disturb the existing import block. If the linter flags duplicate-crate `use`, merge the names into the existing block instead.)

- [ ] **Step 2: Add the `php_versions` command**

Append to `src-tauri/src/commands.rs`:

```rust
#[tauri::command]
pub fn php_versions(state: tauri::State<AppState>) -> Result<Vec<PhpVersionInfo>, String> {
    let config = Config::load(&state.paths.config_file()).unwrap_or_default();
    Ok(core_php_versions(&state.paths, &config.php_version))
}
```

- [ ] **Step 3: Add the `install_php_version` command**

Append to `src-tauri/src/commands.rs`:

```rust
#[tauri::command]
pub async fn install_php_version(
    app: tauri::AppHandle,
    version: String,
) -> Result<Vec<PhpVersionInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<PhpVersionInfo>, String> {
        let state = app.state::<AppState>();
        install_php(&version, &PkexecPrivileged).map_err(|e| e.to_string())?;
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();
        Ok(core_php_versions(&state.paths, &config.php_version))
    })
    .await
    .map_err(|e| e.to_string())?
}
```

- [ ] **Step 4: Add the `set_php_version` command**

Append to `src-tauri/src/commands.rs`:

```rust
#[tauri::command]
pub async fn set_php_version(
    app: tauri::AppHandle,
    version: String,
) -> Result<Vec<ServiceStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<ServiceStatus>, String> {
        let state = app.state::<AppState>();
        if !list_php_fpm_versions(&[state.paths.bin()]).contains(&version) {
            return Err(PhpVersionError::NotInstalled(version).to_string());
        }
        let mut config = Config::load(&state.paths.config_file()).unwrap_or_default();
        config.php_version = version.clone();
        config.save(&state.paths.config_file()).map_err(|e| e.to_string())?;

        let mut orch = state.orch.lock().map_err(lock_err)?;
        orch.replace_php_version(&version).map_err(|e| e.to_string())?;
        Ok(orch.snapshot())
    })
    .await
    .map_err(|e| e.to_string())?
}
```

- [ ] **Step 5: Register the commands in main.rs**

In `src-tauri/src/main.rs`, add to `generate_handler!` after `commands::update_proxy,`:

```rust
            commands::php_versions,
            commands::install_php_version,
            commands::set_php_version,
```

- [ ] **Step 6: Build the app**

Run: `cargo build -p laralux-desktop`
Expected: PASS — compiles cleanly. If a newly imported name is unused, remove it to keep the build warning-clean.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(desktop): php_versions/install_php_version/set_php_version commands"
```

---

### Task 5: Frontend — PHP version card in Settings

**Files:**
- Modify: `dist/app.js`

**Interfaces:**
- Consumes: `php_versions()`, `install_php_version({version})`, `set_php_version({version})`.
- Produces: UI only.

- [ ] **Step 1: Add state**

In `dist/app.js`, in the `state` object, add after `proxy: {...},`:

```js
    phpVersions: [],
    phpBusy: false,
```

- [ ] **Step 2: Add the load + action helpers**

After `refresh()` (or near the other async helpers), add:

```js
  async function loadPhpVersions() {
    try {
      const v = await invoke("php_versions");
      state.phpVersions = Array.isArray(v) ? v : [];
      render();
    } catch (_) { /* settings-only; stay quiet */ }
  }

  async function usePhp(version) {
    if (state.phpBusy) return;
    state.phpBusy = true; render();
    try {
      const arr = await invoke("set_php_version", { version });
      applyServices(arr);
      toast({ type: "success", title: "PHP " + version + " is now active" });
      await loadPhpVersions();
    } catch (e) {
      toast({ type: "error", title: "Switch failed", msg: String(e) });
    } finally {
      state.phpBusy = false; render();
    }
  }

  async function installPhp(version) {
    if (state.phpBusy) return;
    state.phpBusy = true; render();
    try {
      const v = await invoke("install_php_version", { version });
      state.phpVersions = Array.isArray(v) ? v : [];
      toast({ type: "success", title: "PHP " + version + " installed", msg: "Click Use to activate it" });
    } catch (e) {
      toast({ type: "error", title: "Install failed", msg: String(e) });
    } finally {
      state.phpBusy = false; render();
    }
  }
```

- [ ] **Step 3: Load versions when entering Settings**

Find `setView` and change it to load PHP versions on the Settings view:

```js
  function setView(v) {
    state.view = v;
    render();
    if (v === "settings") loadPhpVersions();
  }
```

- [ ] **Step 4: Render the PHP version card in `settingsView`**

In `settingsView`, build the rows and insert a new card before the `settings-foot` line. Replace the `return (` body so it includes the PHP card:

```js
    const phpRows = state.phpVersions.map((p) => {
      let right;
      if (p.active) right = '<span class="tag ok">Active</span>';
      else if (p.installed) right = '<button class="btn-sm" data-action="use-php" data-version="' + esc(p.version) + '"' + (state.phpBusy ? " disabled" : "") + ">Use</button>";
      else right = '<button class="btn-sm" data-action="install-php" data-version="' + esc(p.version) + '"' + (state.phpBusy ? " disabled" : "") + ">Install</button>";
      return '<div class="set-row"><div class="grow"><div class="t">PHP ' + esc(p.version) + '</div><div class="h">' + (p.installed ? "Installed" : "Not installed") + "</div></div>" + right + "</div>";
    }).join("");
    const phpCard =
      '<div class="card settings-card">' +
      '<div class="set-row"><div class="grow"><div class="t">PHP version</div>' +
      '<div class="h">Active version for the stack · install via ondrej PPA (apt)</div></div></div>' +
      (phpRows || '<div class="set-row"><div class="h">Loading…</div></div>') +
      "</div>";
```

Then insert `phpCard` into the returned markup, immediately before the `'<div class="settings-foot">...'` line:

```js
      "</div>" +          // end of the existing settings-card
      phpCard +
      '<div class="settings-foot">Laralux Linux · window 900×600 · min 720×480 · tray: Start All · Stop All · Dashboard · Quit</div>' +
      "</div>"
```

- [ ] **Step 5: Wire the click handlers**

In the delegated click handler, add (after the `open-url` case):

```js
    else if (a === "use-php") usePhp(el.getAttribute("data-version"));
    else if (a === "install-php") installPhp(el.getAttribute("data-version"));
```

- [ ] **Step 6: Syntax-check the JS**

Run: `node --check dist/app.js`
Expected: PASS — exit 0, no output. (If `node` is missing, use `$HOME/.nvm/versions/node/v24.16.0/bin/node --check dist/app.js`.)

- [ ] **Step 7: Manual verification (live)**

Run: `cargo run -p laralux-desktop`, open **Settings** → the **PHP version** card lists 7.4–8.5 with the active one badged. Click **Install** on an uninstalled version (a pkexec prompt + apt run; needs the ondrej PPA to be reachable) → it becomes "Installed". Click **Use** on an installed, non-active version → toast "PHP <v> is now active", and if php-fpm was running it restarts on the new version. (No JS test runner — human-verified.)

- [ ] **Step 8: Commit**

```bash
git add dist/app.js
git commit -m "feat(desktop): PHP version card in Settings (install/switch)"
```

---

## Self-Review

**1. Spec coverage:**
- §3.1 `list_php_fpm_versions` → Task 1. ✓
- §3.2 `php_versions` module (KNOWN 7.4–8.5, PhpVersionInfo, php_versions(_from), apt_packages_for_php Laralux-parity, install_php with ppa+apt+disable, PhpVersionError) → Task 2. ✓
- §3.3 `Orchestrator::replace_php_version` (stop/swap/restart-if-running, returns was_running) → Task 3. ✓
- §3.4 IPC php_versions/install_php_version/set_php_version (+ reject non-installed, persist config, register) → Task 4. ✓
- §3.5 frontend Settings card (list, Active badge, Use/Install, busy-disable, toasts, load on Settings) → Task 5. ✓
- §4 decisions (active vs running, install doesn't auto-switch, reject non-installed, PPA-unavailable surfaced) → Task 3 restart-if-running + Task 4 reject + Task 2 error mapping. ✓
- §5 error handling (typed → toast) → Task 2 errors + Task 4 mapping + Task 5 toasts. ✓
- §6 testing → Tasks 1–3 unit tests; Task 5 node-check + manual. ✓

**2. Placeholder scan:** No TBD/TODO. Every code step shows full code. The one manual step (Task 5 Step 7) is an explicit human-verified UI gate (no JS test runner; apt/pkexec/PPA can't run in tests).

**3. Type consistency:**
- `list_php_fpm_versions(&[PathBuf]) -> Vec<String>` (Task 1) consumed by `php_versions` (Task 2) and `set_php_version` (Task 4). ✓
- `php_versions(&LaraluxPaths, &str) -> Vec<PhpVersionInfo>` (Task 2) aliased `core_php_versions` and called in Task 4. ✓
- `install_php(&str, &dyn Privileged)` (Task 2) called in Task 4 with `PkexecPrivileged`. ✓
- `PhpVersionError::NotInstalled` (Task 2) used by `set_php_version` (Task 4). ✓
- `replace_php_version(&str) -> Result<bool, ServiceError>` (Task 3) called in Task 4. ✓
- IPC arg name `version` and command names match the JS `invoke(...)` calls (Task 5). ✓
- `PhpVersionInfo { version, installed, active }` serialized to JS and read as `p.version/p.installed/p.active` (Task 5). ✓

**Note:** Task 2 Step 1 adds a `fail_apt` toggle to the shared `FakePrivileged`; its default is `false`, so existing sync/setup tests that use `FakePrivileged` are unaffected. The simulated failure uses `PrivError::Command(String)`, the existing string-carrying variant in `privileged.rs`.
