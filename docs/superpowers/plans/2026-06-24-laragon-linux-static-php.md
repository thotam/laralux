# Static PHP Binaries (Phase 2, Version slice 1b) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Source every PHP version from prebuilt static php-fpm binaries (dl.static-php.dev `bulk`) downloaded into `~/laragon/bin`, replacing the apt/ondrej path so PHP install/switch works on any distro with no root.

**Architecture:** New `core::php_static` module resolves a minor version to its newest patch from the `bulk` directory JSON, downloads + extracts the `php-fpm` binary into `~/laragon/bin/php-fpm<minor>` (via the existing `Downloader`/`CommandRunner` seams, no privilege). Detection/switch/UI from the prior slice are reused unchanged. The IPC install path and the Setup wizard switch to static; the apt/ondrej PHP code is removed.

**Tech Stack:** Rust (laragon-core, zero Tauri deps; add `serde_json`), Tauri 2, vanilla JS frontend.

## Global Constraints

- `core` keeps **zero Tauri deps**.
- Commit messages MUST NOT contain a `Co-Authored-By` trailer.
- TDD: failing test first, watch it fail, implement, watch it pass, commit.
- Source: `https://dl.static-php.dev/static-php-cli/bulk/php-<X.Y.Z>-fpm-linux-<arch>.tar.gz`; directory index `…/bulk/?format=json` is a JSON **array** of objects with a `name` field. The fpm tarball contains exactly one file `php-fpm`.
- Versions: `KNOWN_PHP_VERSIONS = ["8.0","8.1","8.2","8.3","8.4","8.5"]` (no 7.4). Default Setup version `DEFAULT_PHP_VERSION = "8.5"`.
- Arch: `x86_64`, `aarch64` only.
- The static binary is installed as `~/laragon/bin/php-fpm<minor>` (e.g. `php-fpm8.4`), mode `0o755`. No pkexec for PHP install/switch.
- Run core tests with `cargo test -p laragon-core`; build with `cargo build -p laragon-desktop`. If `cargo`/`node` aren't on PATH use `$HOME/.cargo/bin/cargo` and `$HOME/.nvm/versions/node/v24.16.0/bin/node`.

Each task must end with a clean `cargo build` (the removal of the apt path is sequenced so every task compiles).

---

### Task 1: `core::php_static` module (resolve + download + install)

**Files:**
- Create: `core/src/php_static.rs`
- Modify: `core/Cargo.toml` (add `serde_json`)
- Modify: `core/src/lib.rs` (declare module + re-exports)

**Interfaces:**
- Consumes: `LaragonPaths`, `setup::Downloader`, `scaffold::CommandRunner`.
- Produces:
  - `pub fn arch_tag() -> Option<&'static str>`
  - `pub fn latest_patch_url(version: &str, arch: &str, listing_json: &str) -> Option<String>`
  - `pub fn install_php_static(paths: &LaragonPaths, version: &str, downloader: &dyn Downloader, runner: &dyn CommandRunner) -> Result<(), PhpStaticError>`
  - `pub enum PhpStaticError { Arch(String), Unavailable(String), Download(String), Extract(String), Io(std::io::Error) }`
  - `pub const STATIC_PHP_BASE: &str`

- [ ] **Step 1: Add `serde_json` to core**

In `core/Cargo.toml` under `[dependencies]`, add:

```toml
serde_json = "1"
```

- [ ] **Step 2: Write the failing tests**

Create `core/src/php_static.rs` with imports + the test module first:

```rust
use crate::paths::LaragonPaths;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;
use std::path::Path;

pub const STATIC_PHP_BASE: &str = "https://dl.static-php.dev/static-php-cli/bulk";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scaffold::ScaffoldError;
    use std::sync::{Arc, Mutex};

    const SAMPLE: &str = r#"[
      {"name":"license/","is_dir":true},
      {"name":"php-8.3.31-fpm-linux-x86_64.tar.gz"},
      {"name":"php-8.4.9-fpm-linux-x86_64.tar.gz"},
      {"name":"php-8.4.22-fpm-linux-x86_64.tar.gz"},
      {"name":"php-8.4.22-cli-linux-x86_64.tar.gz"},
      {"name":"php-8.4.30-fpm-linux-aarch64.tar.gz"}
    ]"#;

    #[test]
    fn latest_patch_url_picks_highest_patch_for_arch() {
        let url = latest_patch_url("8.4", "x86_64", SAMPLE).unwrap();
        assert_eq!(url, format!("{STATIC_PHP_BASE}/php-8.4.22-fpm-linux-x86_64.tar.gz"));
    }

    #[test]
    fn latest_patch_url_none_for_missing_version_or_arch() {
        assert!(latest_patch_url("7.4", "x86_64", SAMPLE).is_none());
        assert!(latest_patch_url("8.4", "riscv64", SAMPLE).is_none());
    }

    #[test]
    fn arch_tag_maps_known() {
        // arch_from is the pure mapping behind arch_tag()
        assert_eq!(arch_from("x86_64"), Some("x86_64"));
        assert_eq!(arch_from("aarch64"), Some("aarch64"));
        assert_eq!(arch_from("riscv64"), None);
    }

    // A downloader that serves the index JSON for the `?format=json` URL and
    // dummy bytes for the tarball URL.
    struct StubDownloader {
        index_json: String,
        fetched: Arc<Mutex<Vec<String>>>,
    }
    impl Downloader for StubDownloader {
        fn fetch(&self, url: &str, dest: &Path) -> std::io::Result<()> {
            self.fetched.lock().unwrap().push(url.to_string());
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if url.ends_with("?format=json") {
                std::fs::write(dest, self.index_json.as_bytes())?;
            } else {
                std::fs::write(dest, b"tarball")?;
            }
            Ok(())
        }
    }

    // A runner that emulates `tar -xzf <tarball> -C <dir> php-fpm` by creating
    // the extracted `php-fpm` file in the dest dir.
    struct TarRunner {
        calls: Arc<Mutex<Vec<(String, Vec<String>)>>>,
    }
    impl CommandRunner for TarRunner {
        fn run(&self, program: &str, args: &[String], _cwd: Option<&Path>) -> Result<(), ScaffoldError> {
            self.calls.lock().unwrap().push((program.to_string(), args.to_vec()));
            // args: ["-xzf", <tarball>, "-C", <dir>, "php-fpm"]
            let dir = &args[3];
            std::fs::write(Path::new(dir).join("php-fpm"), b"bin").unwrap();
            Ok(())
        }
    }

    #[test]
    fn install_php_static_downloads_extracts_and_places_binary() {
        let root = std::env::temp_dir().join(format!("lara-spi-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = LaragonPaths::new(root.clone());
        paths.ensure_dirs().unwrap();
        let arch = arch_tag().expect("supported test arch");
        let json = format!(
            "[{{\"name\":\"php-8.4.22-fpm-linux-{arch}.tar.gz\"}},{{\"name\":\"php-8.4.9-fpm-linux-{arch}.tar.gz\"}}]"
        );
        let fetched = Arc::new(Mutex::new(Vec::new()));
        let dl = StubDownloader { index_json: json, fetched: fetched.clone() };
        let calls = Arc::new(Mutex::new(Vec::new()));
        let runner = TarRunner { calls: calls.clone() };

        install_php_static(&paths, "8.4", &dl, &runner).unwrap();

        let f = fetched.lock().unwrap();
        assert!(f[0].ends_with("?format=json"), "index fetched first");
        assert!(f[1].ends_with("php-8.4.22-fpm-linux-{arch}.tar.gz".replace("{arch}", arch).as_str()));
        assert_eq!(calls.lock().unwrap()[0].0, "tar");
        let bin = paths.bin().join("php-fpm8.4");
        assert!(bin.is_file(), "binary placed in ~/laragon/bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&bin).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn install_php_static_unavailable_version_errors() {
        let root = std::env::temp_dir().join(format!("lara-spi2-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = LaragonPaths::new(root.clone());
        paths.ensure_dirs().unwrap();
        let dl = StubDownloader { index_json: "[]".to_string(), fetched: Arc::new(Mutex::new(Vec::new())) };
        let runner = TarRunner { calls: Arc::new(Mutex::new(Vec::new())) };
        assert!(matches!(
            install_php_static(&paths, "8.4", &dl, &runner),
            Err(PhpStaticError::Unavailable(_))
        ));
        std::fs::remove_dir_all(&root).ok();
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p laragon-core php_static`
Expected: FAIL to compile — module items not defined.

- [ ] **Step 4: Implement the module**

Add above the `#[cfg(test)]` block in `core/src/php_static.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum PhpStaticError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("php {0} is not available as a static build")]
    Unavailable(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Pure arch mapping (testable without touching the host).
fn arch_from(arch: &str) -> Option<&'static str> {
    match arch {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("aarch64"),
        _ => None,
    }
}

/// The static-php arch tag for the current host, or None if unsupported.
pub fn arch_tag() -> Option<&'static str> {
    arch_from(std::env::consts::ARCH)
}

/// Find the newest `php-<version>.<patch>-fpm-linux-<arch>.tar.gz` in the
/// directory JSON and return its full download URL (or None if absent).
pub fn latest_patch_url(version: &str, arch: &str, listing_json: &str) -> Option<String> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(listing_json).ok()?;
    let prefix = format!("php-{version}.");
    let suffix = format!("-fpm-linux-{arch}.tar.gz");
    let mut best: Option<(u32, String)> = None;
    for e in &entries {
        let name = match e.get("name").and_then(|n| n.as_str()) {
            Some(n) => n,
            None => continue,
        };
        if let (true, true) = (name.starts_with(&prefix), name.ends_with(&suffix)) {
            let mid = &name[prefix.len()..name.len() - suffix.len()];
            if let Ok(patch) = mid.parse::<u32>() {
                if best.as_ref().map_or(true, |(b, _)| patch > *b) {
                    best = Some((patch, name.to_string()));
                }
            }
        }
    }
    best.map(|(_, name)| format!("{STATIC_PHP_BASE}/{name}"))
}

/// Download a static php-fpm `bulk` build for `version` and install it as
/// `~/laragon/bin/php-fpm<version>` (mode 0755). No privilege required.
pub fn install_php_static(
    paths: &LaragonPaths,
    version: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
) -> Result<(), PhpStaticError> {
    let arch = arch_tag().ok_or_else(|| PhpStaticError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    std::fs::create_dir_all(paths.bin())?;

    // 1. Fetch the directory index and resolve the newest patch URL.
    let index = paths.tmp().join("static-php-index.json");
    downloader
        .fetch(&format!("{STATIC_PHP_BASE}/?format=json"), &index)
        .map_err(|e| PhpStaticError::Download(e.to_string()))?;
    let json = std::fs::read_to_string(&index)?;
    let url = latest_patch_url(version, arch, &json)
        .ok_or_else(|| PhpStaticError::Unavailable(version.to_string()))?;

    // 2. Download + extract the single `php-fpm` binary into tmp.
    let tarball = paths.tmp().join(format!("php-{version}-fpm.tar.gz"));
    downloader
        .fetch(&url, &tarball)
        .map_err(|e| PhpStaticError::Download(e.to_string()))?;
    runner
        .run(
            "tar",
            &[
                "-xzf".to_string(),
                tarball.display().to_string(),
                "-C".to_string(),
                paths.tmp().display().to_string(),
                "php-fpm".to_string(),
            ],
            None,
        )
        .map_err(|e| PhpStaticError::Extract(e.to_string()))?;

    // 3. Move into place as php-fpm<version> with exec perms.
    let extracted = paths.tmp().join("php-fpm");
    let dest = paths.bin().join(format!("php-fpm{version}"));
    std::fs::rename(&extracted, &dest).or_else(|_| {
        std::fs::copy(&extracted, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(&extracted))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}
```

- [ ] **Step 5: Declare module + re-exports**

In `core/src/lib.rs`, add `pub mod php_static;` (near `pub mod php_versions;`) and:

```rust
pub use php_static::{install_php_static, PhpStaticError};
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p laragon-core php_static`
Expected: PASS — all five tests green.

- [ ] **Step 7: Commit**

```bash
git add core/Cargo.toml core/src/php_static.rs core/src/lib.rs Cargo.lock
git commit -m "feat(core): static php-fpm install from dl.static-php.dev (bulk)"
```

---

### Task 2: IPC install switches to static

**Files:**
- Modify: `src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `install_php_static`, `CurlDownloader`, `RealCommandRunner`, `list_php_fpm_versions`.
- Produces: `install_php_version` uses static download; `set_php_version` rejects uninstalled with a plain string.

- [ ] **Step 1: Update imports**

In `src-tauri/src/commands.rs`, replace the PHP-version import line. Remove `install_php` and `PhpVersionError` from the `use laragon_core::{...}` block and add `install_php_static`. The catalog import `php_versions as core_php_versions`, `list_php_fpm_versions`, and `PhpVersionInfo` stay. Ensure `CurlDownloader` and `RealCommandRunner` are imported (both already used elsewhere in this file).

- [ ] **Step 2: Rewrite `install_php_version`**

Replace the existing `install_php_version` command body with the static path:

```rust
#[tauri::command]
pub async fn install_php_version(
    app: tauri::AppHandle,
    version: String,
) -> Result<Vec<PhpVersionInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<PhpVersionInfo>, String> {
        let state = app.state::<AppState>();
        laragon_core::install_php_static(&state.paths, &version, &CurlDownloader, &RealCommandRunner)
            .map_err(|e| e.to_string())?;
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();
        Ok(core_php_versions(&state.paths, &config.php_version))
    })
    .await
    .map_err(|e| e.to_string())?
}
```

- [ ] **Step 3: De-`PhpVersionError` `set_php_version`**

In `set_php_version`, replace the rejection that used `PhpVersionError::NotInstalled(version).to_string()` with a plain string so the `PhpVersionError` import can be dropped:

```rust
        if !list_php_fpm_versions(&[state.paths.bin()]).contains(&version) {
            return Err(format!("PHP {version} is not installed"));
        }
```

(The rest of `set_php_version` and `php_versions` are unchanged.)

- [ ] **Step 4: Build**

Run: `cargo build -p laragon-desktop`
Expected: PASS — compiles cleanly. (`php_versions::install_php` is now unused by the desktop but still defined in core; removed in Task 4.) If an import is unused, remove just that name.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "feat(desktop): install PHP via static binary (no apt/pkexec)"
```

---

### Task 3: Setup wizard installs PHP statically

**Files:**
- Modify: `core/src/setup.rs`
- Modify: `src-tauri/src/commands.rs` (`run_setup_cmd` passes a runner)
- Modify: `laragonctl/src/main.rs` (pass a runner)

**Interfaces:**
- Consumes: `install_php_static`, `scaffold::{CommandRunner, RealCommandRunner}`, `php_versions::DEFAULT_PHP_VERSION`.
- Produces: `run_setup(paths, privileged, downloader, runner)`; PHP installed via static; `apt_packages_for(Php)` empty; `stack_units_to_disable` drops the php unit.

- [ ] **Step 1: Add `DEFAULT_PHP_VERSION`**

In `core/src/php_versions.rs`, add near `KNOWN_PHP_VERSIONS`:

```rust
pub const DEFAULT_PHP_VERSION: &str = "8.5";
```

- [ ] **Step 2: Write/adjust the failing setup tests**

In `core/src/setup.rs` tests, replace the php-package assertions. Add:

```rust
    #[test]
    fn stack_units_exclude_php() {
        assert_eq!(
            stack_units_to_disable(),
            vec!["nginx".to_string(), "mariadb".to_string(), "redis-server".to_string()]
        );
    }

    #[test]
    fn apt_packages_for_php_is_empty() {
        assert!(apt_packages_for(Component::Php).is_empty());
    }
```

(If an existing test asserts `php_packages` from `classify_apt` or `stack_units_to_disable(Some("8.4"))`, update or remove it to match the new no-arg `stack_units_to_disable()` and the empty PHP apt set.)

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p laragon-core setup`
Expected: FAIL — `stack_units_to_disable` still takes an arg / `apt_packages_for(Php)` not empty / `run_setup` arity.

- [ ] **Step 4: Empty the PHP apt arm and simplify `stack_units_to_disable`**

In `core/src/setup.rs`:
- Change `apt_packages_for`'s `Component::Php` arm to `Component::Php => Vec::new(),`.
- Replace `stack_units_to_disable` with a no-arg version:

```rust
fn stack_units_to_disable() -> Vec<String> {
    vec!["nginx".to_string(), "mariadb".to_string(), "redis-server".to_string()]
}
```

- [ ] **Step 5: Rework `run_setup` to install PHP statically**

Change the signature and body in `core/src/setup.rs`:

```rust
pub fn run_setup(
    paths: &LaragonPaths,
    privileged: &dyn Privileged,
    downloader: &dyn Downloader,
    runner: &dyn crate::scaffold::CommandRunner,
) -> SetupReport {
```

Replace the apt-PHP block (the `let (other_packages, php_packages) = classify_apt(&missing);` section through the php detect/persist) with: install the core apt packages, then install PHP statically when missing.

```rust
    // 1. Install missing apt components (core stack only; PHP is static, below).
    let apt_packages: Vec<String> =
        missing.iter().flat_map(|&c| apt_packages_for(c)).collect();
    report.apt_packages = apt_packages.clone();
    if !apt_packages.is_empty() {
        if let Err(e) = privileged.apt_install(&apt_packages) {
            report.errors.push(format!("apt_install: {e}"));
        }
    }

    // 1b. Install PHP from a static build (no apt/distro PHP) when missing.
    if missing.contains(&Component::Php) {
        match crate::php_static::install_php_static(
            paths,
            crate::php_versions::DEFAULT_PHP_VERSION,
            downloader,
            runner,
        ) {
            Ok(()) => match crate::bin::detect_php_fpm_version(&[paths.bin()]) {
                Some(ver) => {
                    report.php_version = Some(ver.clone());
                    let mut cfg = crate::config::Config::load(&paths.config_file()).unwrap_or_default();
                    cfg.php_version = ver;
                    if let Err(e) = cfg.save(&paths.config_file()) {
                        report.errors.push(format!("persist php version: {e}"));
                    }
                }
                None => report
                    .errors
                    .push("php-fpm binary not found after static install".to_string()),
            },
            Err(e) => report.errors.push(format!("install php (static): {e}")),
        }
    }
```

Then delete the now-unused `classify_apt` function. Update the systemd-disable call to the no-arg form:

```rust
    let stack_units = stack_units_to_disable();
    if let Err(e) = privileged.disable_system_services(&stack_units) {
        report.errors.push(format!("disable system services: {e}"));
    }
```

(The mailpit step and the rest of `run_setup` are unchanged.)

- [ ] **Step 6: Update the two callers**

In `src-tauri/src/commands.rs` `run_setup_cmd`:

```rust
        Ok(run_setup(&state.paths, &privileged, &downloader, &RealCommandRunner))
```

In `laragonctl/src/main.rs` (where `run_setup(&paths, &SudoPrivileged, &CurlDownloader)` is called), pass a runner:

```rust
            let report = run_setup(&paths, &SudoPrivileged, &CurlDownloader, &RealCommandRunner);
```

Add `RealCommandRunner` to laragonctl's `use laragon_core::{...}` imports if not present.

- [ ] **Step 7: Run tests + build**

Run: `cargo test -p laragon-core setup` then `cargo build -p laragon-desktop && cargo build -p laragonctl`
Expected: PASS — setup tests green; both binaries compile.

- [ ] **Step 8: Commit**

```bash
git add core/src/setup.rs core/src/php_versions.rs src-tauri/src/commands.rs laragonctl/src/main.rs
git commit -m "feat(core): Setup installs PHP from static binary instead of apt"
```

---

### Task 4: Remove the dead apt/ondrej PHP code

**Files:**
- Modify: `core/src/php_versions.rs`
- Modify: `core/src/privileged.rs`
- Modify: `core/src/lib.rs` (drop removed re-exports)

**Interfaces:**
- Consumes: nothing new.
- Produces: `KNOWN_PHP_VERSIONS = ["8.0".."8.5"]`; `apt_packages_for_php`, `ondrej_suite`, `ondrej_suite_for`, `install_php` (apt), `PhpVersionError` removed from `php_versions`; `add_ondrej_php` (trait + impls + helper + Fake recorder) and `fail_apt`/`set_fail_apt` removed from `privileged`.

- [ ] **Step 1: Trim `php_versions.rs`**

In `core/src/php_versions.rs`:
- Change `KNOWN_PHP_VERSIONS` to `["8.0", "8.1", "8.2", "8.3", "8.4", "8.5"]` (length 6).
- Delete `apt_packages_for_php`, `ondrej_suite`, `ondrej_suite_for`, the `const ONDREJ_LTS`, and the apt `install_php` function.
- Delete `enum PhpVersionError` (no longer referenced).
- In the `tests` module, delete `apt_packages_are_laragon_parity`, `ondrej_suite_falls_back_to_newest_lts`, `install_php_adds_ondrej_installs_and_disables_unit`, and `install_php_surfaces_apt_error`. Keep `php_versions_marks_installed_and_active` and `php_versions_includes_unknown_installed` (both still valid for 8.0–8.5).
- Remove the now-unused `use crate::privileged::Privileged;` import if present.

- [ ] **Step 2: Trim `privileged.rs`**

In `core/src/privileged.rs`:
- Remove the `fn add_ondrej_php(&self, suite: &str) -> Result<(), PrivError>;` line from the `Privileged` trait.
- Remove the `ondrej_php_argv` helper.
- Remove the `add_ondrej_php` impls from `SudoPrivileged` and `PkexecPrivileged`.
- In `FakePrivileged`: remove the `ondrej_suites` field, its `ondrej_suites()` accessor, its `add_ondrej_php` impl, and the `fail_apt` field + `set_fail_apt` method; revert `apt_install` to the simple recording form:

```rust
    fn apt_install(&self, packages: &[String]) -> Result<(), PrivError> {
        self.apt_installs.lock().unwrap().push(packages.to_vec());
        Ok(())
    }
```

- [ ] **Step 3: Fix lib.rs re-exports**

In `core/src/lib.rs`, ensure the `php_versions` re-export no longer names removed items. It should read:

```rust
pub use php_versions::{php_versions, PhpVersionInfo};
```

(Drop `install_php` and `PhpVersionError`; `KNOWN_PHP_VERSIONS`/`DEFAULT_PHP_VERSION` are used via `crate::php_versions::` internally and need no re-export.)

- [ ] **Step 4: Run the whole suite + build**

Run: `cargo test -p laragon-core` then `cargo build -p laragon-desktop && cargo build -p laragonctl`
Expected: PASS — all core tests green; no unused-code warnings for the removed items; both binaries build.

- [ ] **Step 5: Commit**

```bash
git add core/src/php_versions.rs core/src/privileged.rs core/src/lib.rs
git commit -m "refactor(core): remove apt/ondrej PHP path (static-only)"
```

---

### Task 5: Frontend — Install copy reflects a download

**Files:**
- Modify: `dist/app.js`

**Interfaces:**
- Consumes: unchanged IPC.
- Produces: UI copy only.

- [ ] **Step 1: Update the Install toast copy**

In `dist/app.js` `installPhp`, change the success/progress wording to reflect a download rather than apt. Update the success toast message:

```js
      toast({ type: "success", title: "PHP " + version + " installed", msg: "Downloaded · click Use to activate" });
```

If `installPhp` shows any "apt"/"Installing via apt" text, change it to "Downloading…". (The card structure and handlers are otherwise unchanged.)

- [ ] **Step 2: Syntax-check the JS**

Run: `node --check dist/app.js`
Expected: PASS — exit 0, no output.

- [ ] **Step 3: Manual verification (live)**

Run: `cargo run -p laragon-desktop`, open **Settings → PHP version**, click **Install** on a not-installed version (e.g. 8.4) → it downloads from dl.static-php.dev into `~/laragon/bin/php-fpm8.4` (no pkexec prompt) and the row becomes Installed. Click **Use** → toast "PHP 8.4 is now active"; if php-fpm was running it restarts on 8.4. (No JS test runner — human-verified.)

- [ ] **Step 4: Commit**

```bash
git add dist/app.js
git commit -m "feat(desktop): PHP Install copy reflects static download"
```

---

## Self-Review

**1. Spec coverage:**
- §3.1 `php_static` (arch_tag, latest_patch_url, install_php_static, PhpStaticError, STATIC_PHP_BASE, serde_json) → Task 1. ✓
- §3.2 catalog 8.0–8.5 + remove apt helpers/PhpVersionError → Task 4 (KNOWN/removals) with `DEFAULT_PHP_VERSION` added in Task 3. ✓
- §3.3 remove `add_ondrej_php`/`fail_apt` from privileged → Task 4. ✓
- §3.4 Setup static PHP + `run_setup(runner)` + callers + apt(Php) empty + stack_units → Task 3. ✓
- §3.5 IPC install_php_version static; set_php_version string reject → Task 2. ✓
- §3.6 frontend copy → Task 5. ✓
- §6 testing: arch/latest_patch_url/install_php_static (Task 1); setup no-php-apt + stack_units (Task 3); catalog (Task 4); node-check + manual (Task 5). ✓

**2. Placeholder scan:** No TBD/TODO. Every code step shows complete code. The single manual step (Task 5 Step 3) is an explicit human-verified gate (network download + live php-fpm restart can't run in unit tests).

**3. Type consistency:**
- `install_php_static(&LaragonPaths, &str, &dyn Downloader, &dyn CommandRunner) -> Result<(), PhpStaticError>` (Task 1) called by IPC (Task 2) and Setup (Task 3) with matching args. ✓
- `Downloader::fetch(&self, url: &str, dest: &Path) -> std::io::Result<()>` (existing) matches the StubDownloader and the install body. ✓
- `CommandRunner::run(&self, &str, &[String], Option<&Path>) -> Result<(), ScaffoldError>` (existing) matches the tar call + TarRunner. ✓
- `run_setup(paths, privileged, downloader, runner)` (Task 3) matches both updated callers. ✓
- `latest_patch_url(version, arch, json)` URL format `"{STATIC_PHP_BASE}/{name}"` matches the test's expected URL. ✓
- Sequencing keeps every task compiling: static added (T1) → IPC switched (T2) → setup switched (T3) → dead code removed (T4) → frontend (T5). ✓

**Note:** Task 1's `install_php_static` test fakes (`StubDownloader`/`TarRunner`) are module-local because the shared `FakeDownloader` always writes `b"fake"` (invalid JSON) and `FakeCommandRunner` does not create the extracted file. `paths.bin()` is created by `install_php_static` before the move, so the binary lands even on a fresh root.
