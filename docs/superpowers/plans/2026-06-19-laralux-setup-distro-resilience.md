# Laralux — Plan 3c: Distro-Resilient PHP Setup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the setup wizard install PHP from the distro's own repositories (no ondrej PPA), detect whichever PHP version got installed, and persist it — so setup works on Ubuntu releases the ondrej PPA doesn't support (e.g. 26.04 "resolute").

**Architecture:** Acceptance testing showed the ondrej PPA returns 404 on "resolute", leaving a broken apt source that makes `apt-get update` fail and (because the install command was `update && install`) blocks the *entire* stack install. Fix: drop the ondrej PPA, install unversioned PHP meta packages (`php-fpm`, `php-cli`, …) that exist on every Ubuntu and pull the distro default, make `apt-get update` non-fatal so a broken third-party repo can't block installs, detect the actual installed `php-fpm<ver>` binary, and persist that version into `laralux.toml` so the orchestrator spawns the right binary on next launch.

**Tech Stack:** Rust, reuses `laralux_core` (Plans 1–3b) + Tauri 2; live tools: apt, pkexec/sudo.

## Global Constraints

- Do NOT add the ondrej PPA (it 404s on Ubuntu releases it doesn't build for). Install PHP from distro repos via unversioned meta packages: `php-fpm php-cli php-mysql php-curl php-mbstring php-xml`.
- The escalated apt command MUST tolerate a failing `apt-get update` (broken third-party repo) and still attempt the install: `sh -c "apt-get update || true; apt-get install -y <pkgs>"`.
- PHP version is detected at install time from the installed `php-fpm<major>.<minor>` binary and persisted to `Config::php_version` in `~/laralux/laralux.toml`. The orchestrator already builds `PhpFpmService` from `Config::php_version`, so the spawned binary + per-site vhost match after the next app launch.
- Detection of "PHP present" = any `php-fpm<X.Y>` resolvable (not a hardcoded version).
- `detect`, `apt_packages_for`, and `run_setup` drop their `php_version` parameter (no longer needed). `AppState` drops its `php_version` field. `Config::php_version` stays (used by `build_services`).
- After a first successful setup the user must restart the app so the orchestrator picks up the persisted PHP version — the GUI/CLI must say so.
- `core` keeps zero Tauri deps. Commit messages MUST NOT contain a `Co-Authored-By` trailer.
- TDD for `core` changes (Tasks 1–2). Caller/UI rewiring (Task 3) is build + manual smoke.

---

### Task 1: Detect installed php-fpm version

**Files:**
- Modify: `core/src/bin.rs` (add `detect_php_fpm_version` + a private `parse_php_version`)

**Interfaces:**
- Consumes: the existing private `FALLBACK_DIRS` const in `bin.rs`.
- Produces: `bin::detect_php_fpm_version(extra_dirs: &[PathBuf]) -> Option<String>` — scans `extra_dirs`, `$PATH`, and the fallback dirs for files named `php-fpm<major>.<minor>` and returns the highest `<major>.<minor>` as a string (e.g. `"8.4"`); `None` if none found.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `core/src/bin.rs`:

```rust
    #[test]
    fn detects_highest_php_fpm_version() {
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("php-fpm8.3"), "x").unwrap();
        std::fs::write(dir.join("php-fpm8.4"), "x").unwrap();
        std::fs::write(dir.join("php-fpm"), "x").unwrap(); // unversioned: ignored
        assert_eq!(detect_php_fpm_version(&[dir.clone()]), Some("8.4".to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_php_fpm_returns_none() {
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(detect_php_fpm_version(&[dir.clone()]), None);
        std::fs::remove_dir_all(&dir).ok();
    }
```

(The `tmp()` helper already exists in `bin.rs`'s test module from Plan 3b Task 1.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laralux-core bin::tests::detects_highest_php_fpm_version`
Expected: FAIL — `cannot find function detect_php_fpm_version`.

- [ ] **Step 3: Write minimal implementation**

Add to `core/src/bin.rs` (after `resolve_or_name`):

```rust
/// Parse "8.4" → (8, 4). Returns None if not exactly major.minor of integers.
fn parse_php_version(s: &str) -> Option<(u32, u32)> {
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor))
}

/// Find the highest installed `php-fpm<major>.<minor>` and return its version string.
pub fn detect_php_fpm_version(extra_dirs: &[PathBuf]) -> Option<String> {
    let mut dirs: Vec<PathBuf> = extra_dirs.to_vec();
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.extend(FALLBACK_DIRS.iter().map(PathBuf::from));

    let mut best: Option<(u32, u32, String)> = None;
    for dir in dirs {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(ver) = name.strip_prefix("php-fpm") {
                if let Some((maj, min)) = parse_php_version(ver) {
                    if best.as_ref().map_or(true, |(bmaj, bmin, _)| (maj, min) > (*bmaj, *bmin)) {
                        best = Some((maj, min, ver.to_string()));
                    }
                }
            }
        }
    }
    best.map(|(_, _, ver)| ver)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p laralux-core bin`
Expected: PASS — the existing bin tests plus the 2 new ones.

- [ ] **Step 5: Commit**

```bash
git add core/src/bin.rs
git commit -m "feat(core): detect installed php-fpm version"
```

---

### Task 2: Distro-resilient run_setup (drop ondrej, unversioned PHP, persist detected version)

**Files:**
- Modify: `core/src/privileged.rs` (`apt_argv` non-fatal update)
- Modify: `core/src/setup.rs` (`apt_packages_for`/`detect`/`run_setup` drop `php_version`; unversioned PHP; no ondrej; persist version; `SetupReport.php_version`)

**Interfaces:**
- Consumes: `bin::detect_php_fpm_version`, `Config::{load,save,php_version}`, `Privileged::apt_install` (the `add_apt_repository` method stays in the trait but is no longer called by `run_setup`).
- Produces:
  - `apt_argv(packages)` → `["sh", "-c", "apt-get update || true; apt-get install -y <pkgs>"]`.
  - `apt_packages_for(component: Component) -> Vec<String>` (no `php_version` param); `Php` → `["php-fpm","php-cli","php-mysql","php-curl","php-mbstring","php-xml"]`.
  - `detect(paths: &LaraluxPaths) -> Vec<ComponentStatus>` (no `php_version` param); `Php` present iff `bin::detect_php_fpm_version(&[paths.bin()]).is_some()`.
  - `SetupReport` gains `pub php_version: Option<String>`.
  - `run_setup(paths: &LaraluxPaths, privileged: &dyn Privileged, downloader: &dyn Downloader) -> SetupReport` (no `php_version` param) — installs the non-PHP apt set and the unversioned PHP set in two separate `apt_install` calls (no PPA); after the PHP install, detects the php-fpm version, sets `report.php_version`, and persists it to `Config::php_version` in `laralux.toml`.

- [ ] **Step 1: Update apt_argv + its test (privileged.rs)**

Change `apt_argv` in `core/src/privileged.rs` to:

```rust
fn apt_argv(packages: &[String]) -> Vec<String> {
    vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("apt-get update || true; apt-get install -y {}", packages.join(" ")),
    ]
}
```

Update the existing test `apt_argv_builds_update_then_install` in `privileged.rs` to assert the non-fatal update:

```rust
    #[test]
    fn apt_argv_builds_update_then_install() {
        let argv = apt_argv(&["nginx".to_string(), "redis-server".to_string()]);
        assert_eq!(argv[0], "sh");
        assert_eq!(argv[1], "-c");
        assert!(argv[2].contains("apt-get update || true"));
        assert!(argv[2].contains("apt-get install -y nginx redis-server"));
    }
```

- [ ] **Step 2: Run the privileged test to verify it passes**

Run: `cargo test -p laralux-core privileged::tests::apt_argv_builds_update_then_install`
Expected: PASS (the `|| true` is now present).

- [ ] **Step 3: Write the failing setup test**

Replace the body of `run_setup_installs_missing_apt_and_fetches_mailpit` in `core/src/setup.rs` with the new contract (no PPA; two installs; persisted version). Also update the Task-3 (Plan 3b) tests that call `apt_packages_for(..., "8.4")` / `detect(&paths, "8.4")` to the new no-version signatures:

```rust
    #[test]
    fn php_packages_are_unversioned_meta() {
        let pkgs = apt_packages_for(Component::Php);
        assert!(pkgs.contains(&"php-fpm".to_string()));
        assert!(pkgs.contains(&"php-mysql".to_string()));
        assert!(!pkgs.iter().any(|p| p.contains('8'))); // no hardcoded version
    }

    #[test]
    fn mailpit_has_no_apt_packages() {
        assert!(apt_packages_for(Component::Mailpit).is_empty());
    }

    #[test]
    fn mkcert_includes_nss_tools() {
        let pkgs = apt_packages_for(Component::Mkcert);
        assert!(pkgs.contains(&"mkcert".to_string()));
        assert!(pkgs.contains(&"libnss3-tools".to_string()));
    }

    #[test]
    fn detect_reports_all_components() {
        let paths = LaraluxPaths::new(std::env::temp_dir().join(format!("lara-detect-{}", std::process::id())));
        let statuses = detect(&paths);
        assert_eq!(statuses.len(), 6);
    }

    #[test]
    fn run_setup_installs_core_and_php_without_ppa() {
        let root = std::env::temp_dir().join(format!("lara-runsetup-{}", std::process::id()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let paths = LaraluxPaths::new(root.clone());

        let priv_ = FakePrivileged::new();
        let apt_log = priv_.apt_installs();
        let add_repos = priv_.add_repos();
        let dl = FakeDownloader::new();
        let urls = dl.requested();

        let report = run_setup(&paths, &priv_, &dl);

        // No PPA is added anymore.
        assert!(add_repos.lock().unwrap().is_empty());
        // Two apt installs: core (has nginx, no php) + php (unversioned meta).
        let calls = apt_log.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let core = calls.iter().find(|c| c.iter().any(|p| p == "nginx")).unwrap();
        assert!(core.iter().any(|p| p == "mariadb-server"));
        assert!(!core.iter().any(|p| p.starts_with("php")));
        let php = calls.iter().find(|c| c.iter().all(|p| p.starts_with("php"))).unwrap();
        assert!(php.iter().any(|p| p == "php-fpm"));
        // mailpit fetched, mkcert CA attempted.
        assert!(urls.lock().unwrap().iter().any(|u| u.contains("mailpit")));
        assert!(report.mkcert_ca);
        std::fs::remove_dir_all(&root).ok();
    }
```

- [ ] **Step 4: Run setup tests to verify they fail**

Run: `cargo test -p laralux-core setup`
Expected: FAIL — `apt_packages_for`/`detect`/`run_setup` still take a `php_version` argument (arity mismatch) and `SetupReport.php_version` is missing.

- [ ] **Step 5: Update setup.rs implementation**

In `core/src/setup.rs`:

(a) Change `detect` to drop the param and use the version detector for PHP:

```rust
pub fn detect(paths: &LaraluxPaths) -> Vec<ComponentStatus> {
    Component::ALL
        .iter()
        .map(|&component| {
            let present = match component {
                Component::Php => crate::bin::detect_php_fpm_version(&[paths.bin()]).is_some(),
                other => {
                    let name = detect_binary(other);
                    resolve_bin(&name, &[paths.bin()]).is_some()
                }
            };
            ComponentStatus { component, present }
        })
        .collect()
}
```

(b) Change `detect_binary` to drop the `php_version` param and the `Php` arm (now handled in `detect`):

```rust
fn detect_binary(component: Component) -> String {
    match component {
        Component::Nginx => "nginx".to_string(),
        Component::Php => "php-fpm".to_string(), // unused for detection (handled in detect)
        Component::Mariadb => "mariadbd".to_string(),
        Component::Redis => "redis-server".to_string(),
        Component::Mkcert => "mkcert".to_string(),
        Component::Mailpit => "mailpit".to_string(),
    }
}
```

(c) Change `apt_packages_for` to drop the param and use unversioned PHP meta:

```rust
pub fn apt_packages_for(component: Component) -> Vec<String> {
    match component {
        Component::Nginx => vec!["nginx".to_string()],
        Component::Php => vec![
            "php-fpm".to_string(),
            "php-cli".to_string(),
            "php-mysql".to_string(),
            "php-curl".to_string(),
            "php-mbstring".to_string(),
            "php-xml".to_string(),
        ],
        Component::Mariadb => vec!["mariadb-server".to_string()],
        Component::Redis => vec!["redis-server".to_string()],
        Component::Mkcert => vec!["mkcert".to_string(), "libnss3-tools".to_string()],
        Component::Mailpit => Vec::new(),
    }
}
```

(d) Add `php_version` to `SetupReport`:

```rust
#[derive(Clone, Debug, Serialize)]
pub struct SetupReport {
    pub apt_packages: Vec<String>,
    pub mailpit_fetched: bool,
    pub mkcert_ca: bool,
    pub nginx_setcap: bool,
    pub php_version: Option<String>,
    pub errors: Vec<String>,
}
```

(e) Rewrite `run_setup` (drop `php_version` param; no PPA; two installs; detect + persist). Replace the whole function with:

```rust
pub fn run_setup(
    paths: &LaraluxPaths,
    privileged: &dyn Privileged,
    downloader: &dyn Downloader,
) -> SetupReport {
    let mut report = SetupReport {
        apt_packages: Vec::new(),
        mailpit_fetched: false,
        mkcert_ca: false,
        nginx_setcap: false,
        php_version: None,
        errors: Vec::new(),
    };
    let _ = paths.ensure_dirs();
    let statuses = detect(paths);
    let missing: Vec<Component> =
        statuses.iter().filter(|s| !s.present).map(|s| s.component).collect();

    // 1. Install missing apt components: core stack in one call, PHP (unversioned
    //    distro meta) in a separate call so one failing package can't block the rest.
    let other_packages: Vec<String> = missing
        .iter()
        .filter(|c| !matches!(c, Component::Php))
        .flat_map(|&c| apt_packages_for(c))
        .collect();
    let php_packages: Vec<String> = missing
        .iter()
        .filter(|c| matches!(c, Component::Php))
        .flat_map(|&c| apt_packages_for(c))
        .collect();
    report.apt_packages = other_packages.iter().chain(php_packages.iter()).cloned().collect();

    if !other_packages.is_empty() {
        if let Err(e) = privileged.apt_install(&other_packages) {
            report.errors.push(format!("apt_install (core): {e}"));
        }
    }
    if !php_packages.is_empty() {
        if let Err(e) = privileged.apt_install(&php_packages) {
            report.errors.push(format!("apt_install (php): {e}"));
        }
        // Detect the version that actually got installed and persist it.
        if let Some(ver) = crate::bin::detect_php_fpm_version(&[paths.bin()]) {
            report.php_version = Some(ver.clone());
            let mut cfg = crate::config::Config::load(&paths.config_file()).unwrap_or_default();
            cfg.php_version = ver;
            if let Err(e) = cfg.save(&paths.config_file()) {
                report.errors.push(format!("persist php version: {e}"));
            }
        }
    }

    // 2. Fetch + extract mailpit into ~/laralux/bin when missing.
    if missing.contains(&Component::Mailpit) {
        let tarball = paths.tmp().join("mailpit.tar.gz");
        match downloader.fetch(MAILPIT_URL, &tarball) {
            Ok(()) => {
                report.mailpit_fetched = true;
                let output = std::process::Command::new("tar")
                    .arg("-xzf")
                    .arg(&tarball)
                    .arg("-C")
                    .arg(paths.bin())
                    .arg("mailpit")
                    .output();
                match output {
                    Ok(o) if o.status.success() => {}
                    Ok(o) => report.errors.push(format!(
                        "tar extract mailpit failed: {}",
                        String::from_utf8_lossy(&o.stderr).trim()
                    )),
                    Err(e) => report.errors.push(format!("tar spawn: {e}")),
                }
            }
            Err(e) => report.errors.push(format!("mailpit download: {e}")),
        }
    }

    // 3. Install the mkcert local CA (idempotent).
    match privileged.install_mkcert_ca() {
        Ok(()) => report.mkcert_ca = true,
        Err(e) => report.errors.push(format!("mkcert -install: {e}")),
    }

    // 4. setcap the resolved nginx binary (same path the orchestrator spawns).
    if let Some(nginx) = resolve_bin("nginx", &[paths.bin()]) {
        match privileged.setcap_nginx(&nginx) {
            Ok(()) => report.nginx_setcap = true,
            Err(e) => report.errors.push(format!("setcap nginx: {e}")),
        }
    }

    report
}
```

- [ ] **Step 6: Run the full core suite**

Run: `cargo test -p laralux-core`
Expected: PASS — all tests green (the run_setup test no longer expects a PPA; signatures updated). Note: `Privileged::add_apt_repository` and its tests remain (the method is still part of the trait); only `run_setup`'s call to it is removed.

- [ ] **Step 7: Commit**

```bash
git add core/src/setup.rs core/src/privileged.rs
git commit -m "fix(core): install distro PHP without ondrej PPA and persist detected version"
```

---

### Task 3: Update callers — GUI + CLI (new signatures, show version, restart hint)

**Files:**
- Modify: `src-tauri/src/commands.rs` (drop `AppState.php_version`; update command bodies)
- Modify: `dist/main.js` (alert shows detected PHP version + restart hint)
- Modify: `laraluxctl/src/main.rs` (update `setup` arm calls; print version + restart hint)

**Interfaces:**
- Consumes: `detect_components(paths)` and `run_setup(paths, privileged, downloader)` (new no-version signatures from Task 2).

- [ ] **Step 1: Update `commands.rs`**

Remove the `php_version` field from `AppState`:

```rust
pub struct AppState {
    pub orch: Mutex<Orchestrator>,
    pub paths: LaraluxPaths,
    pub tld: String,
}
```

In `build_state`, drop the `php_version` initializer:

```rust
    AppState { orch: Mutex::new(orch), paths, tld: config.tld }
```

Update the two setup commands to the new signatures:

```rust
#[tauri::command]
pub fn setup_status(state: tauri::State<AppState>) -> Result<Vec<ComponentStatus>, String> {
    Ok(detect_components(&state.paths))
}

#[tauri::command]
pub fn run_setup_cmd(state: tauri::State<AppState>) -> Result<SetupReport, String> {
    let privileged = PkexecPrivileged;
    let downloader = CurlDownloader;
    Ok(run_setup(&state.paths, &privileged, &downloader))
}
```

- [ ] **Step 2: Update the GUI alert in `dist/main.js`**

Replace the `#run-setup` click handler's success `alert(...)` line so it reports the detected PHP version and the restart requirement:

```javascript
    const report = await invoke("run_setup_cmd");
    const errs = report.errors.length ? `\nErrors:\n${report.errors.join("\n")}` : "";
    const php = report.php_version ? `\nPHP ${report.php_version} installed — restart the app to use it.` : "";
    alert(`Setup done. apt: ${report.apt_packages.join(", ") || "none"}; mkcert CA: ${report.mkcert_ca}; nginx setcap: ${report.nginx_setcap}${php}${errs}`);
```

- [ ] **Step 3: Update the `setup` arm in `laraluxctl/src/main.rs`**

Update the calls to the new signatures and print the version + restart hint:

```rust
        "setup" => {
            paths.ensure_dirs().expect("create dirs");
            println!("Component status:");
            for s in detect_components(&paths) {
                println!("  {:?}: {}", s.component, if s.present { "installed" } else { "missing" });
            }
            println!("Running setup (may prompt for sudo)...");
            let report = run_setup(&paths, &SudoPrivileged, &CurlDownloader);
            println!(
                "apt: {}\nmailpit fetched: {}\nmkcert CA: {}\nnginx setcap: {}",
                if report.apt_packages.is_empty() { "none".to_string() } else { report.apt_packages.join(" ") },
                report.mailpit_fetched, report.mkcert_ca, report.nginx_setcap
            );
            if let Some(ver) = &report.php_version {
                println!("PHP {ver} installed — restart laralux to use it.");
            }
            for e in &report.errors {
                eprintln!("  error: {e}");
            }
        }
```

(The `Config` import may now be unused in this arm — if `cargo build` warns about an unused import, remove `Config` from the `use laralux_core::{...}` line only if no other arm uses it; the `config-init`/`up`/`status`/`sites` arms DO use `Config`, so keep it.)

- [ ] **Step 4: Build both binaries**

Run: `cargo build -p laralux-desktop && cargo build -p laraluxctl`
Expected: PASS — both compile with the new signatures, no unused-variable warnings.

- [ ] **Step 5: Run the full workspace suite**

Run: `cargo test --workspace`
Expected: PASS — all core tests green; both bins build.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands.rs dist/main.js laraluxctl/src/main.rs
git commit -m "feat(desktop,cli): use distro PHP setup, show detected version and restart hint"
```

- [ ] **Step 7: Manual smoke (human, live)**

First remove any broken ondrej source left by the previous build: `sudo rm -f /etc/apt/sources.list.d/ondrej-ubuntu-php-resolute.sources && sudo apt-get update`. Then `cargo run -p laralux-desktop` → **Install missing**: setup adds no PPA, installs nginx/mariadb/redis/mkcert + the distro PHP meta, detects the PHP version, and the alert reports it with a restart hint. After restarting the app, **Start All** brings services to `Running` and `https://demo.dev` opens. Record this as a human verification step.

---

## Self-Review

**1. Coverage of the reported failure:**
- ondrej PPA 404 on resolute → broken apt source → blocked install: fixed by dropping the PPA (Task 2 run_setup no longer calls `add_apt_repository`) and unversioned distro PHP (Task 2 `apt_packages_for`). ✓
- `apt-get update && install` letting a broken repo block installs: fixed by `apt-get update || true; apt-get install` (Task 2 `apt_argv`). ✓
- Hardcoded php8.4 not available: fixed by unversioned meta + version detection + persistence (Tasks 1, 2). ✓
- Orchestrator spawning the wrong php-fpm version: persisted `Config::php_version` + restart hint means the next launch's `build_services` uses the detected version (Tasks 2, 3). ✓

**2. Placeholder scan:** No "TBD/handle edge cases". The unused-import note in Task 3 Step 3 is conditional and concrete (keep `Config`, it's used by other arms). Live steps are flagged human.

**3. Type consistency:** `detect_php_fpm_version(&[PathBuf]) -> Option<String>` (T1) used by `detect` and `run_setup` (T2). `detect(paths)`, `apt_packages_for(component)`, `run_setup(paths, privileged, downloader)` — the dropped `php_version` param is consistent across the definitions (T2) and every caller (T3: GUI commands, CLI). `SetupReport.php_version: Option<String>` (T2) read by `main.js` (`report.php_version`) and the CLI (T3). `AppState` without `php_version` (T3) — `build_state` and both commands updated to match. `Config::php_version` retained and written by `run_setup`, read by `build_services` at launch. `add_apt_repository` stays in the trait (Plan 3b tests still pass); only its call site is removed.

**Note:** the orphan-on-SIGKILL / dnsmasq / idempotency-gating items from earlier final reviews remain tracked for a later hardening pass; out of scope here.
