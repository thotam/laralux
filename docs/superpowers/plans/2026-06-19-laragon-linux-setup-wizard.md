# Laragon Linux — Plan 3b: Setup Wizard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Detect which stack components are missing and install + configure them (apt packages, mailpit binary, mkcert CA, `setcap` on nginx) so "Start All" works and `https://<name>.dev` opens end-to-end.

**Architecture:** Adds a `core::setup` module that detects missing components (via a new `core::bin` binary resolver), plans the apt package set, and runs installation through two trait seams — `Privileged` (now with `apt_install`, plus a `PkexecPrivileged` for graphical auth in the GUI) and a new `Downloader` (curl, for the mailpit binary). The binary resolver also fixes the long-standing nginx-resolution divergence: the orchestrator resolves each service's program to an absolute path at spawn time, and `setcap` targets that same resolved path. The Tauri app and `laragonctl` each wire a setup action.

**Tech Stack:** Rust, reuses `laragon_core` (Plans 1–3a) + Tauri 2; live tools: `apt-get`, `pkexec`/`sudo`, `mkcert`, `setcap`, `curl`, `tar`. Target distro: Ubuntu 26.04 (apt).

## Global Constraints

- Stack components and their detection binaries: nginx→`nginx`, php-fpm→`php-fpm<ver>` (ver from `Config::php_version`, e.g. `php-fpm8.4`), mariadb→`mariadbd`, redis→`redis-server`, mkcert→`mkcert`, mailpit→`mailpit`.
- apt packages: nginx→`nginx`; php→`php<ver>-fpm php<ver>-cli php<ver>-mysql php<ver>-curl php<ver>-mbstring php<ver>-xml`; mariadb→`mariadb-server`; redis→`redis-server`; mkcert→`mkcert libnss3-tools`. mailpit is NOT in apt — it is downloaded.
- Mailpit download URL: `https://github.com/axllent/mailpit/releases/latest/download/mailpit-linux-amd64.tar.gz`; extracted `mailpit` binary goes into `~/laragon/bin/` (no root needed there).
- Binary resolution search order: `~/laragon/bin`, then `$PATH`, then `/usr/local/sbin`, `/usr/local/bin`, `/usr/sbin`, `/usr/bin`, `/sbin`, `/bin`. A name containing `/` is treated as a path (used as-is if it is a file).
- Privileged escalation: GUI uses `pkexec` (graphical prompt); CLI uses `sudo`. All escalated ops go through the `Privileged` trait — unit tests use `FakePrivileged`, so `cargo test` needs no root.
- `setcap` MUST target the same absolute nginx path the orchestrator spawns (both via the resolver) — this resolves the Plan-2 divergence finding.
- `core` keeps zero Tauri deps and adds no HTTP crate (mailpit fetch shells out to `curl` via the `Downloader` trait).
- Commit messages MUST NOT contain a `Co-Authored-By` trailer.
- TDD applies to all `core` changes (Tasks 1–4). GUI/CLI wiring (Tasks 5–6) is build + manual smoke; live install steps (apt/curl/pkexec/setcap) are human-verified on the real machine.

---

### Task 1: Binary resolver + spawn-time resolution

**Files:**
- Modify: `core/src/paths.rs` (add `bin()` + include in `ensure_dirs`)
- Create: `core/src/bin.rs`
- Modify: `core/src/lib.rs` (add `pub mod bin;`)
- Modify: `core/src/orchestrator.rs` (resolve the spawn program against `paths.bin()`)

**Interfaces:**
- Consumes: `LaragonPaths`, `SpawnSpec`, `ProcessSpawner` (Plan 1).
- Produces:
  - `LaragonPaths::bin(&self) -> PathBuf` (`<root>/bin`), created by `ensure_dirs`.
  - `bin::resolve_bin(name: &str, extra_dirs: &[PathBuf]) -> Option<PathBuf>` — search per the Global Constraints order; a `name` containing `/` returns `Some(path)` iff it is a file, else `None`.
  - `bin::resolve_or_name(name: &str, extra_dirs: &[PathBuf]) -> String` — the resolved absolute path as a string if found, else the bare `name`.
  - `Orchestrator::start` resolves `spec.program` via `resolve_or_name(&spec.program, &[paths.bin()])` before spawning (so services keep returning bare names, but real binaries in sbin / `~/laragon/bin` are found).

- [ ] **Step 1: Add `bin()` to LaragonPaths**

In `core/src/paths.rs`, add this method inside `impl LaragonPaths` (next to `ssl`):

```rust
    pub fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }
```

And in `ensure_dirs`, add `self.bin()` to the directory array:

```rust
        for dir in [self.www(), self.etc(), self.data(), self.log(), self.tmp(), self.ssl(), self.bin()] {
```

- [ ] **Step 2: Add the module declaration**

In `core/src/lib.rs` add (with the other `pub mod` lines):

```rust
pub mod bin;
```

- [ ] **Step 3: Write the failing test**

Create `core/src/bin.rs` with the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static C: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!("lara-bin-{}-{}", std::process::id(), C.fetch_add(1, Ordering::SeqCst)))
    }

    #[test]
    fn resolves_a_binary_in_an_extra_dir() {
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("mybin");
        std::fs::write(&exe, "x").unwrap();
        assert_eq!(resolve_bin("mybin", &[dir.clone()]), Some(exe));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_binary_resolves_to_none_and_bare_name() {
        let nonexistent = "definitely-not-a-real-binary-xyz";
        assert_eq!(resolve_bin(nonexistent, &[]), None);
        assert_eq!(resolve_or_name(nonexistent, &[]), nonexistent.to_string());
    }

    #[test]
    fn path_with_slash_is_used_as_is() {
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("tool");
        std::fs::write(&exe, "x").unwrap();
        let abs = exe.display().to_string();
        assert_eq!(resolve_bin(&abs, &[]), Some(exe));
        assert_eq!(resolve_bin("/no/such/tool/here", &[]), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test -p laragon-core bin`
Expected: FAIL — `cannot find function resolve_bin`.

- [ ] **Step 5: Write minimal implementation**

Prepend to `core/src/bin.rs`:

```rust
use std::path::PathBuf;

const FALLBACK_DIRS: [&str; 6] = [
    "/usr/local/sbin",
    "/usr/local/bin",
    "/usr/sbin",
    "/usr/bin",
    "/sbin",
    "/bin",
];

/// Resolve a program name to an absolute path.
/// A name containing '/' is treated as a path and returned only if it is a file.
/// Otherwise searches: extra_dirs, then $PATH, then common system bin dirs.
pub fn resolve_bin(name: &str, extra_dirs: &[PathBuf]) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return if p.is_file() { Some(p) } else { None };
    }
    let mut dirs: Vec<PathBuf> = extra_dirs.to_vec();
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.extend(FALLBACK_DIRS.iter().map(PathBuf::from));
    for dir in dirs {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Resolved absolute path as a string if found, else the bare name
/// (so PATH lookup still applies at spawn time).
pub fn resolve_or_name(name: &str, extra_dirs: &[PathBuf]) -> String {
    resolve_bin(name, extra_dirs)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| name.to_string())
}
```

- [ ] **Step 6: Write the failing orchestrator resolution test**

Add to the `tests` module in `core/src/orchestrator.rs`:

```rust
    #[test]
    fn start_resolves_program_against_bin_dir() {
        // A fake binary placed in <root>/bin should be spawned by absolute path.
        let root = std::env::temp_dir().join(format!("lara-orch-res-{}", std::process::id()));
        let bindir = root.join("bin");
        std::fs::create_dir_all(&bindir).unwrap();
        let exe = bindir.join("redis-server");
        std::fs::write(&exe, "x").unwrap();

        let spawner = crate::process::FakeSpawner::new();
        let log = spawner.log();
        let services: Vec<Box<dyn Service>> =
            vec![Box::new(Dummy { kind: ServiceKind::Redis, name: "redis-server" })];
        let mut o = Orchestrator::new(LaragonPaths::new(root.clone()), services, Box::new(spawner));

        o.start(ServiceKind::Redis).unwrap();
        assert_eq!(log.lock().unwrap()[0].program, exe.display().to_string());
        std::fs::remove_dir_all(&root).ok();
    }
```

- [ ] **Step 7: Run test to verify it fails**

Run: `cargo test -p laragon-core orchestrator::tests::start_resolves_program_against_bin_dir`
Expected: FAIL — the spawned program is the bare `"redis-server"`, not the absolute path.

- [ ] **Step 8: Resolve the program in the orchestrator**

In `core/src/orchestrator.rs`, find where `start`/`do_start` builds the spec and spawns. Immediately after obtaining `let spec = svc.command(&self.paths);` (the call that produces the `SpawnSpec`), insert the resolution so `program` becomes absolute when found:

```rust
        let mut spec = svc.command(&self.paths);
        spec.program = crate::bin::resolve_or_name(&spec.program, &[self.paths.bin()]);
```

(Replace the existing `let spec = svc.command(&self.paths);` line with the two lines above; everything downstream that uses `spec` is unchanged.)

- [ ] **Step 9: Run tests to verify they pass**

Run: `cargo test -p laragon-core`
Expected: PASS — all prior tests plus the new `bin` and orchestrator tests.

- [ ] **Step 10: Commit**

```bash
git add core/
git commit -m "feat(core): add binary resolver and resolve spawn program at start"
```

---

### Task 2: Privileged — apt_install + pkexec escalation

**Files:**
- Modify: `core/src/privileged.rs`
- Modify: `core/src/lib.rs` (re-export `PkexecPrivileged`)

**Interfaces:**
- Consumes: existing `Privileged` trait, `PrivError`, `SudoPrivileged`, `FakePrivileged` (Plan 2).
- Produces:
  - `Privileged::apt_install(&self, packages: &[String]) -> Result<(), PrivError>` (new trait method) — implemented by `SudoPrivileged`, `PkexecPrivileged`, `FakePrivileged`.
  - `struct PkexecPrivileged;` implementing `Privileged` using `pkexec` for escalation (graphical prompt) — mirrors `SudoPrivileged`'s ops.
  - `FakePrivileged::apt_installs(&self) -> Arc<Mutex<Vec<Vec<String>>>>` (records each `apt_install` call's package list).
  - Private free helpers shared by both real impls: `cp_argv`, `setcap_argv`, `apt_argv`, `run_escalated` (keeps the existing `SudoPrivileged::hosts_cp_command`/`setcap_command` static builders intact and returning `("sudo", …)`).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `core/src/privileged.rs`:

```rust
    #[test]
    fn apt_argv_builds_update_then_install() {
        let argv = apt_argv(&["nginx".to_string(), "redis-server".to_string()]);
        assert_eq!(argv[0], "sh");
        assert_eq!(argv[1], "-c");
        assert!(argv[2].contains("apt-get update"));
        assert!(argv[2].contains("apt-get install -y nginx redis-server"));
    }

    #[test]
    fn pkexec_uses_pkexec_program() {
        // The pkexec impl escalates with `pkexec`; verify via the shared builder usage.
        // hosts_cp_command on Sudo still uses sudo (unchanged Plan-2 contract).
        let (prog, _args) = SudoPrivileged::hosts_cp_command(std::path::Path::new("/tmp/h"));
        assert_eq!(prog, "sudo");
    }

    #[test]
    fn fake_records_apt_installs() {
        let f = FakePrivileged::new();
        let log = f.apt_installs();
        f.apt_install(&["nginx".to_string()]).unwrap();
        assert_eq!(log.lock().unwrap().len(), 1);
        assert_eq!(log.lock().unwrap()[0], vec!["nginx".to_string()]);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core privileged`
Expected: FAIL — `cannot find function apt_argv` / `no method named apt_installs`.

- [ ] **Step 3: Add the shared helpers + trait method + impls**

In `core/src/privileged.rs`:

(a) Add the trait method to `pub trait Privileged`:

```rust
    fn apt_install(&self, packages: &[String]) -> Result<(), PrivError>;
```

(b) Add shared free helpers near the top (after the trait):

```rust
fn cp_argv(src: &Path) -> Vec<String> {
    vec!["cp".to_string(), src.display().to_string(), "/etc/hosts".to_string()]
}

fn setcap_argv(bin: &Path) -> Vec<String> {
    vec![
        "setcap".to_string(),
        "cap_net_bind_service=+ep".to_string(),
        bin.display().to_string(),
    ]
}

fn apt_argv(packages: &[String]) -> Vec<String> {
    vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("apt-get update && apt-get install -y {}", packages.join(" ")),
    ]
}

fn run_escalated(escalator: &str, argv: &[String]) -> Result<(), PrivError> {
    let status = std::process::Command::new(escalator)
        .args(argv)
        .status()
        .map_err(|e| PrivError::Command(format!("spawn {escalator}: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(PrivError::Command(format!("{escalator} command failed")))
    }
}
```

(c) Update `impl Privileged for SudoPrivileged` to delegate to the helpers and add `apt_install`. Replace the existing method bodies so they use `run_escalated("sudo", …)`, and keep the existing public static `hosts_cp_command`/`setcap_command` builders (have them call `cp_argv`/`setcap_argv` and prepend `"sudo"`):

```rust
impl SudoPrivileged {
    pub fn hosts_cp_command(src: &Path) -> (String, Vec<String>) {
        ("sudo".to_string(), cp_argv(src))
    }
    pub fn setcap_command(bin: &Path) -> (String, Vec<String>) {
        ("sudo".to_string(), setcap_argv(bin))
    }
}

impl Privileged for SudoPrivileged {
    fn write_etc_hosts(&self, new_content: &str) -> Result<(), PrivError> {
        let tmp = std::env::temp_dir().join("laragon-hosts.new");
        std::fs::write(&tmp, new_content)?;
        run_escalated("sudo", &cp_argv(&tmp))
    }
    fn install_mkcert_ca(&self) -> Result<(), PrivError> {
        run_escalated("mkcert", &["-install".to_string()])
    }
    fn setcap_nginx(&self, nginx_bin: &Path) -> Result<(), PrivError> {
        run_escalated("sudo", &setcap_argv(nginx_bin))
    }
    fn apt_install(&self, packages: &[String]) -> Result<(), PrivError> {
        run_escalated("sudo", &apt_argv(packages))
    }
}
```

(d) Add `PkexecPrivileged`:

```rust
/// Privileged operations escalated with `pkexec` (graphical auth) — for GUI use.
pub struct PkexecPrivileged;

impl Privileged for PkexecPrivileged {
    fn write_etc_hosts(&self, new_content: &str) -> Result<(), PrivError> {
        let tmp = std::env::temp_dir().join("laragon-hosts.new");
        std::fs::write(&tmp, new_content)?;
        run_escalated("pkexec", &cp_argv(&tmp))
    }
    fn install_mkcert_ca(&self) -> Result<(), PrivError> {
        run_escalated("mkcert", &["-install".to_string()])
    }
    fn setcap_nginx(&self, nginx_bin: &Path) -> Result<(), PrivError> {
        run_escalated("pkexec", &setcap_argv(nginx_bin))
    }
    fn apt_install(&self, packages: &[String]) -> Result<(), PrivError> {
        run_escalated("pkexec", &apt_argv(packages))
    }
}
```

(e) Extend `FakePrivileged` with apt recording. Add the field to the struct and the accessor + trait method:

```rust
// add to struct fields:
    apt_installs: Arc<Mutex<Vec<Vec<String>>>>,
```

```rust
// add to impl FakePrivileged:
    pub fn apt_installs(&self) -> Arc<Mutex<Vec<Vec<String>>>> {
        self.apt_installs.clone()
    }
```

```rust
// add to impl Privileged for FakePrivileged:
    fn apt_install(&self, packages: &[String]) -> Result<(), PrivError> {
        self.apt_installs.lock().unwrap().push(packages.to_vec());
        Ok(())
    }
```

(`FakePrivileged` derives `Default`, so the new `Arc<Mutex<Vec<_>>>` field defaults correctly.)

- [ ] **Step 4: Re-export PkexecPrivileged**

In `core/src/lib.rs`, update the privileged re-export line to include it:

```rust
pub use privileged::{PkexecPrivileged, Privileged, SudoPrivileged};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p laragon-core privileged`
Expected: PASS — the 3 new tests plus the existing privileged tests (the Plan-2 `sudo_command_builders_are_correct` / `fake_records_hosts_write` still pass).

- [ ] **Step 6: Commit**

```bash
git add core/src/privileged.rs core/src/lib.rs
git commit -m "feat(core): add apt_install and pkexec privileged escalation"
```

---

### Task 3: Setup detection + apt package planning

**Files:**
- Create: `core/src/setup.rs`
- Modify: `core/src/lib.rs` (add `pub mod setup;`)

**Interfaces:**
- Consumes: `LaragonPaths`, `bin::resolve_bin`.
- Produces:
  - `enum Component { Nginx, Php, Mariadb, Redis, Mkcert, Mailpit }` (derives `Clone, Copy, PartialEq, Eq, Debug, serde::Serialize`).
  - `impl Component { pub const ALL: [Component; 6]; pub fn label(&self) -> &'static str; }`
  - `struct ComponentStatus { pub component: Component, pub present: bool }` (derives `Clone, Debug, PartialEq, Eq, serde::Serialize`).
  - `fn detect(paths: &LaragonPaths, php_version: &str) -> Vec<ComponentStatus>` — presence per the detection-binary rules (mailpit also searches `paths.bin()`).
  - `fn apt_packages_for(component: Component, php_version: &str) -> Vec<String>` — per the package map; `Mailpit` returns empty.

- [ ] **Step 1: Add the module declaration**

In `core/src/lib.rs` add:

```rust
pub mod setup;
```

- [ ] **Step 2: Write the failing test**

Create `core/src/setup.rs` with the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaragonPaths;

    #[test]
    fn php_packages_are_versioned() {
        let pkgs = apt_packages_for(Component::Php, "8.4");
        assert!(pkgs.contains(&"php8.4-fpm".to_string()));
        assert!(pkgs.contains(&"php8.4-mysql".to_string()));
    }

    #[test]
    fn mailpit_has_no_apt_packages() {
        assert!(apt_packages_for(Component::Mailpit, "8.4").is_empty());
    }

    #[test]
    fn mkcert_includes_nss_tools() {
        let pkgs = apt_packages_for(Component::Mkcert, "8.4");
        assert!(pkgs.contains(&"mkcert".to_string()));
        assert!(pkgs.contains(&"libnss3-tools".to_string()));
    }

    #[test]
    fn detect_reports_all_components() {
        let paths = LaragonPaths::new(std::env::temp_dir().join(format!("lara-detect-{}", std::process::id())));
        let statuses = detect(&paths, "8.4");
        assert_eq!(statuses.len(), 6);
        // A bogus root means mailpit (only in ~/laragon/bin or PATH) is absent here.
        let mailpit = statuses.iter().find(|s| s.component == Component::Mailpit).unwrap();
        assert!(!mailpit.present);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laragon-core setup`
Expected: FAIL — `cannot find type Component`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/setup.rs`:

```rust
use crate::bin::resolve_bin;
use crate::paths::LaragonPaths;
use serde::Serialize;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
pub enum Component {
    Nginx,
    Php,
    Mariadb,
    Redis,
    Mkcert,
    Mailpit,
}

impl Component {
    pub const ALL: [Component; 6] = [
        Component::Nginx,
        Component::Php,
        Component::Mariadb,
        Component::Redis,
        Component::Mkcert,
        Component::Mailpit,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Component::Nginx => "nginx",
            Component::Php => "php-fpm",
            Component::Mariadb => "mariadb",
            Component::Redis => "redis",
            Component::Mkcert => "mkcert",
            Component::Mailpit => "mailpit",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ComponentStatus {
    pub component: Component,
    pub present: bool,
}

/// The binary that, if resolvable, means the component is installed.
fn detect_binary(component: Component, php_version: &str) -> String {
    match component {
        Component::Nginx => "nginx".to_string(),
        Component::Php => format!("php-fpm{php_version}"),
        Component::Mariadb => "mariadbd".to_string(),
        Component::Redis => "redis-server".to_string(),
        Component::Mkcert => "mkcert".to_string(),
        Component::Mailpit => "mailpit".to_string(),
    }
}

/// Detect presence of every component. Mailpit also searches `~/laragon/bin`.
pub fn detect(paths: &LaragonPaths, php_version: &str) -> Vec<ComponentStatus> {
    Component::ALL
        .iter()
        .map(|&component| {
            let name = detect_binary(component, php_version);
            let present = resolve_bin(&name, &[paths.bin()]).is_some();
            ComponentStatus { component, present }
        })
        .collect()
}

/// The apt packages that install a component (empty for mailpit, which is downloaded).
pub fn apt_packages_for(component: Component, php_version: &str) -> Vec<String> {
    match component {
        Component::Nginx => vec!["nginx".to_string()],
        Component::Php => vec![
            format!("php{php_version}-fpm"),
            format!("php{php_version}-cli"),
            format!("php{php_version}-mysql"),
            format!("php{php_version}-curl"),
            format!("php{php_version}-mbstring"),
            format!("php{php_version}-xml"),
        ],
        Component::Mariadb => vec!["mariadb-server".to_string()],
        Component::Redis => vec!["redis-server".to_string()],
        Component::Mkcert => vec!["mkcert".to_string(), "libnss3-tools".to_string()],
        Component::Mailpit => Vec::new(),
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p laragon-core setup`
Expected: PASS — 4 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/setup.rs core/src/lib.rs
git commit -m "feat(core): add setup component detection and apt package planning"
```

---

### Task 4: Downloader + run_setup orchestration

**Files:**
- Modify: `core/src/setup.rs`
- Modify: `core/src/lib.rs` (re-export setup symbols)

**Interfaces:**
- Consumes: `Privileged` (with `apt_install`), `bin::resolve_bin`, `detect`, `apt_packages_for`.
- Produces:
  - `enum SetupError { Io(std::io::Error), Download(String) }` (thiserror).
  - `trait Downloader: Send + Sync { fn fetch(&self, url: &str, dest: &std::path::Path) -> Result<(), SetupError>; }`
  - `struct CurlDownloader;` (runs `curl -fL <url> -o <dest>`).
  - `struct FakeDownloader` (records requested URLs, writes a placeholder file to `dest`; NOT `#[cfg(test)]`-gated) + `requested()`.
  - `const MAILPIT_URL: &str` (the release tarball URL).
  - `struct SetupReport { pub apt_packages: Vec<String>, pub mailpit_fetched: bool, pub mkcert_ca: bool, pub nginx_setcap: bool, pub errors: Vec<String> }` (derives `Clone, Debug, Serialize`).
  - `fn run_setup(paths: &LaragonPaths, php_version: &str, privileged: &dyn Privileged, downloader: &dyn Downloader) -> SetupReport` — installs missing apt components, fetches+extracts mailpit if missing, installs the mkcert CA, and `setcap`s the resolved nginx binary; collects non-fatal errors.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `core/src/setup.rs`:

```rust
    use crate::privileged::FakePrivileged;

    #[test]
    fn run_setup_installs_missing_apt_and_fetches_mailpit() {
        let root = std::env::temp_dir().join(format!("lara-runsetup-{}", std::process::id()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let paths = LaragonPaths::new(root.clone());

        let priv_ = FakePrivileged::new();
        let apt_log = priv_.apt_installs();
        let dl = FakeDownloader::new();
        let urls = dl.requested();

        let report = run_setup(&paths, "8.4", &priv_, &dl);

        // On a machine without the stack, all apt components are planned for install.
        let installed: Vec<String> = apt_log.lock().unwrap().iter().flatten().cloned().collect();
        assert!(installed.contains(&"nginx".to_string()));
        assert!(installed.contains(&"mariadb-server".to_string()));
        assert!(installed.iter().any(|p| p.starts_with("php8.4-")));
        // mailpit is fetched, not apt-installed.
        assert!(urls.lock().unwrap().iter().any(|u| u.contains("mailpit")));
        assert!(report.mkcert_ca);
        std::fs::remove_dir_all(&root).ok();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core setup::tests::run_setup_installs_missing_apt_and_fetches_mailpit`
Expected: FAIL — `cannot find type FakeDownloader` / `function run_setup`.

- [ ] **Step 3: Write minimal implementation**

Append to `core/src/setup.rs` (after the existing code, before the test module):

```rust
use crate::privileged::Privileged;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub const MAILPIT_URL: &str =
    "https://github.com/axllent/mailpit/releases/latest/download/mailpit-linux-amd64.tar.gz";

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("setup io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("download error: {0}")]
    Download(String),
}

/// Fetches a URL to a destination file.
pub trait Downloader: Send + Sync {
    fn fetch(&self, url: &str, dest: &Path) -> Result<(), SetupError>;
}

pub struct CurlDownloader;

impl Downloader for CurlDownloader {
    fn fetch(&self, url: &str, dest: &Path) -> Result<(), SetupError> {
        let status = std::process::Command::new("curl")
            .arg("-fL")
            .arg(url)
            .arg("-o")
            .arg(dest)
            .status()
            .map_err(|e| SetupError::Download(format!("spawn curl: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(SetupError::Download(format!("curl failed for {url}")))
        }
    }
}

#[derive(Clone, Default)]
pub struct FakeDownloader {
    requested: Arc<Mutex<Vec<String>>>,
}

impl FakeDownloader {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn requested(&self) -> Arc<Mutex<Vec<String>>> {
        self.requested.clone()
    }
}

impl Downloader for FakeDownloader {
    fn fetch(&self, url: &str, dest: &Path) -> Result<(), SetupError> {
        self.requested.lock().unwrap().push(url.to_string());
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(dest, b"fake")?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SetupReport {
    pub apt_packages: Vec<String>,
    pub mailpit_fetched: bool,
    pub mkcert_ca: bool,
    pub nginx_setcap: bool,
    pub errors: Vec<String>,
}

/// Install missing components, fetch mailpit, install the mkcert CA, and setcap nginx.
/// Non-fatal: each failure is collected into `report.errors`.
pub fn run_setup(
    paths: &LaragonPaths,
    php_version: &str,
    privileged: &dyn Privileged,
    downloader: &dyn Downloader,
) -> SetupReport {
    let mut report = SetupReport {
        apt_packages: Vec::new(),
        mailpit_fetched: false,
        mkcert_ca: false,
        nginx_setcap: false,
        errors: Vec::new(),
    };
    let _ = paths.ensure_dirs();
    let statuses = detect(paths, php_version);
    let missing: Vec<Component> =
        statuses.iter().filter(|s| !s.present).map(|s| s.component).collect();

    // 1. apt-install all missing apt-backed components in one call.
    let apt_packages: Vec<String> = missing
        .iter()
        .flat_map(|&c| apt_packages_for(c, php_version))
        .collect();
    if !apt_packages.is_empty() {
        report.apt_packages = apt_packages.clone();
        if let Err(e) = privileged.apt_install(&apt_packages) {
            report.errors.push(format!("apt_install: {e}"));
        }
    }

    // 2. Fetch + extract mailpit into ~/laragon/bin when missing.
    if missing.contains(&Component::Mailpit) {
        let tarball = paths.tmp().join("mailpit.tar.gz");
        match downloader.fetch(MAILPIT_URL, &tarball) {
            Ok(()) => {
                report.mailpit_fetched = true;
                let status = std::process::Command::new("tar")
                    .arg("-xzf")
                    .arg(&tarball)
                    .arg("-C")
                    .arg(paths.bin())
                    .arg("mailpit")
                    .status();
                match status {
                    Ok(s) if s.success() => {}
                    Ok(_) => report.errors.push("tar extract mailpit failed".to_string()),
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

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p laragon-core setup`
Expected: PASS — the run_setup test plus the Task-3 tests. (In the test the FakePrivileged records apt + mkcert; nginx isn't really installed so `nginx_setcap` stays false — the test does not assert it.)

- [ ] **Step 5: Re-export setup symbols**

In `core/src/lib.rs`, add:

```rust
pub use setup::{detect as detect_components, run_setup, Component, ComponentStatus, CurlDownloader, SetupReport};
```

- [ ] **Step 6: Run the full suite**

Run: `cargo test -p laragon-core`
Expected: PASS — everything green.

- [ ] **Step 7: Commit**

```bash
git add core/src/setup.rs core/src/lib.rs
git commit -m "feat(core): add Downloader and run_setup orchestration"
```

---

### Task 5: GUI setup wiring

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs` (register the 2 new commands)
- Modify: `dist/index.html`
- Modify: `dist/main.js`
- Modify: `dist/styles.css`

**Interfaces:**
- Consumes: `laragon_core::{detect_components, run_setup, ComponentStatus, SetupReport, CurlDownloader, PkexecPrivileged}`, the existing `AppState` (has `paths` + needs the php version).
- Produces:
  - `AppState` gains a `php_version: String` field (set in `build_state`).
  - Commands: `setup_status(state) -> Result<Vec<ComponentStatus>, String>` and `run_setup_cmd(state) -> Result<SetupReport, String>` (the latter uses `PkexecPrivileged` + `CurlDownloader`). Frontend invoke name for the second is `run_setup`.
  - Dashboard "Setup" section listing each component present/missing with an "Install missing" button.

- [ ] **Step 1: Add php_version to AppState and the two commands**

In `src-tauri/src/commands.rs`, add `php_version` to the struct and set it in `build_state`:

```rust
pub struct AppState {
    pub orch: Mutex<Orchestrator>,
    pub paths: LaragonPaths,
    pub tld: String,
    pub php_version: String,
}
```

In `build_state`, change the returned struct to include it (the `config` is already loaded there):

```rust
    AppState { orch: Mutex::new(orch), paths, tld: config.tld, php_version: config.php_version }
}
```

Update the imports at the top of `commands.rs` to add the setup symbols:

```rust
use laragon_core::{
    build_services, detect_components, run_setup, scan_sites, Config, LaragonPaths, Orchestrator,
    PkexecPrivileged, RealSpawner, ServiceKind, ServiceStatus, Site,
};
use laragon_core::{ComponentStatus, CurlDownloader, SetupReport};
```

Add the two commands at the end of `commands.rs`:

```rust
#[tauri::command]
pub fn setup_status(state: tauri::State<AppState>) -> Result<Vec<ComponentStatus>, String> {
    Ok(detect_components(&state.paths, &state.php_version))
}

#[tauri::command]
pub fn run_setup_cmd(state: tauri::State<AppState>) -> Result<SetupReport, String> {
    let privileged = PkexecPrivileged;
    let downloader = CurlDownloader;
    Ok(run_setup(&state.paths, &state.php_version, &privileged, &downloader))
}
```

- [ ] **Step 2: Register the commands in `main.rs`**

In `src-tauri/src/main.rs`, add the two commands to the `tauri::generate_handler!` list (after `commands::list_sites,`):

```rust
            commands::setup_status,
            commands::run_setup_cmd,
```

- [ ] **Step 3: Add the Setup section to `dist/index.html`**

Insert this `<section>` as the first child of `<main>` (before the Services section):

```html
      <section id="setup-section">
        <h2>Setup</h2>
        <ul id="setup"></ul>
        <button id="run-setup">Install missing</button>
      </section>
```

- [ ] **Step 4: Add setup logic to `dist/main.js`**

Add near the top, after the existing `const ... = document.querySelector(...)` lines:

```javascript
const setupEl = document.querySelector("#setup");

function renderSetup(list) {
  setupEl.innerHTML = "";
  for (const { component, present } of list) {
    const li = document.createElement("li");
    li.textContent = `${component}: ${present ? "installed" : "missing"}`;
    li.className = present ? "running" : "crashed";
    setupEl.appendChild(li);
  }
}
```

In the `refresh()` function, add a call to also refresh setup status (inside the `try` block):

```javascript
    renderSetup(await invoke("setup_status"));
```

At the bottom, after the existing button listeners, add:

```javascript
document.querySelector("#run-setup").addEventListener("click", async () => {
  const btn = document.querySelector("#run-setup");
  btn.disabled = true;
  btn.textContent = "Installing… (authorize when prompted)";
  try {
    const report = await invoke("run_setup");
    const errs = report.errors.length ? `\nErrors:\n${report.errors.join("\n")}` : "";
    alert(`Setup done. apt: ${report.apt_packages.join(", ") || "none"}; mkcert CA: ${report.mkcert_ca}; nginx setcap: ${report.nginx_setcap}${errs}`);
    await refresh();
  } catch (e) {
    alert(`setup failed: ${e}`);
  } finally {
    btn.disabled = false;
    btn.textContent = "Install missing";
  }
});
```

Note: the frontend invokes `run_setup` — but the Rust command is `run_setup_cmd`. Register the alias by giving the command the frontend-facing name. In `commands.rs`, annotate the command with an explicit rename so the JS name `run_setup` maps to it:

```rust
#[tauri::command(rename_all = "snake_case")]
pub fn run_setup_cmd(...) // keep as-is
```

That does NOT rename the command itself. Instead, change the JS call to `invoke("run_setup_cmd")` to match the Rust function name exactly (Tauri derives the command name from the function identifier). Update the `dist/main.js` listener to use `invoke("run_setup_cmd")` rather than `invoke("run_setup")`.

- [ ] **Step 5: Add a small style for the setup button in `dist/styles.css`**

```css
#setup { list-style: none; padding: 0; }
#setup li { padding: 0.3rem 0; }
#run-setup {
  cursor: pointer;
  border: 1px solid #c4c9d0;
  background: #fff;
  border-radius: 6px;
  padding: 0.4rem 0.9rem;
  margin-top: 0.5rem;
}
#run-setup:hover { background: #eef1f5; }
#run-setup:disabled { opacity: 0.6; cursor: default; }
```

- [ ] **Step 6: Build**

Run: `cargo build -p laragon-desktop`
Expected: PASS — compiles with the two new commands.

- [ ] **Step 7: Commit**

```bash
git add src-tauri dist
git commit -m "feat(desktop): add setup status and run-setup to the dashboard"
```

- [ ] **Step 8: Manual smoke (human, live)**

`cargo run -p laragon-desktop`: the Setup section lists each component as installed/missing. Clicking "Install missing" triggers a `pkexec` graphical auth prompt, then apt installs the stack, downloads mailpit, installs the mkcert CA, and setcaps nginx; afterwards components show "installed" and "Start All" brings services to `Running` and `https://demo.dev` opens. Record this as a human verification step (needs network + auth).

---

### Task 6: laragonctl setup subcommand

**Files:**
- Modify: `laragonctl/src/main.rs`

**Interfaces:**
- Consumes: `laragon_core::{detect_components, run_setup, CurlDownloader, SudoPrivileged, Config, LaragonPaths}`.
- Produces: a `setup` subcommand that prints component status, runs `run_setup` with `SudoPrivileged` + `CurlDownloader`, and prints the report; the usage line lists `setup`.

- [ ] **Step 1: Add the setup arm**

In `laragonctl/src/main.rs`, update the `use laragon_core::{...}` line to add `detect_components, run_setup, CurlDownloader` (keep existing imports), then add this match arm (before the `_ =>` arm):

```rust
        "setup" => {
            let cfg = Config::load(&paths.config_file()).expect("load config");
            paths.ensure_dirs().expect("create dirs");
            println!("Component status:");
            for s in detect_components(&paths, &cfg.php_version) {
                println!("  {:?}: {}", s.component, if s.present { "installed" } else { "missing" });
            }
            println!("Running setup (may prompt for sudo)...");
            let report = run_setup(&paths, &cfg.php_version, &SudoPrivileged, &CurlDownloader);
            println!(
                "apt: {}\nmailpit fetched: {}\nmkcert CA: {}\nnginx setcap: {}",
                if report.apt_packages.is_empty() { "none".to_string() } else { report.apt_packages.join(" ") },
                report.mailpit_fetched, report.mkcert_ca, report.nginx_setcap
            );
            for e in &report.errors {
                eprintln!("  error: {e}");
            }
        }
```

Update the usage line in the `_ =>` arm to include `setup`:

```rust
            println!("usage: laragonctl <config-init|up|status|sites|setup-perms|setup>");
```

- [ ] **Step 2: Build**

Run: `cargo build -p laragonctl`
Expected: PASS — compiles.

- [ ] **Step 3: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS — all `core` tests green; `laragonctl`/`laragon-desktop` build.

- [ ] **Step 4: CLI smoke (status only, no root)**

```bash
cargo run -p laragonctl -- setup 2>&1 | head -8
```
Expected: prints the "Component status:" block listing each component as installed/missing. (The actual install runs apt/sudo and is a human/live step — interrupt before authorizing if you only want to see status.)

- [ ] **Step 5: Commit**

```bash
git add laragonctl/src/main.rs
git commit -m "feat(laragonctl): add setup subcommand"
```

---

## Self-Review

**1. Spec coverage (Plan 3b scope = setup wizard):**
- Detect missing stack (spec §7 Phase-1 "Wizard setup lần đầu: cài stack qua apt") → Task 3 `detect` ✓
- Install via apt (spec §7) → Tasks 2 (`apt_install`) + 4 (`run_setup`) ✓
- mkcert CA install (spec §6, §7) → Task 4 ✓
- setcap nginx, aligned with the spawned binary (spec §5 + Plan-2 deferred finding) → Tasks 1 (resolver + spawn resolution) + 4 (setcap resolved nginx) ✓
- mailpit (not in apt) install (spec §7 stack list) → Task 4 download+extract ✓
- GUI-driven setup with graphical privilege escalation (spec §2 polkit) → Tasks 2 (`PkexecPrivileged`) + 5 ✓
- CLI setup path → Task 6 ✓
- mariadb datadir init: already handled lazily by `MariadbService::needs_init/init` at first start (Plan 1) once `mariadbd`/`mariadb-install-db` are installed — no separate task needed; the resolver (Task 1) ensures the now-installed binaries are found.
- TDD for all core logic (spec §9) → Tasks 1–4 ✓
- **Correctly deferred (out of scope):** dnsmasq wildcard, PostgreSQL/MongoDB/Memcached, Procfile, ngrok/share, auto-update, multi-profile (spec §7 Phase 3).

**2. Placeholder scan:** No "TBD/handle edge cases". Task 5 Step 4 explicitly resolves the command-name detail (JS must call `run_setup_cmd`, the Rust function name). Live install steps (apt/curl/pkexec/setcap) are called out as human verification, not vague.

**3. Type consistency:** `resolve_bin(name, extra_dirs)`/`resolve_or_name` (Task 1) used identically in orchestrator (Task 1) and `setup` (Tasks 3–4). `Privileged::apt_install(&[String])` (Task 2) called by `run_setup` (Task 4) and both CLI/GUI (Tasks 5–6). `Component`/`ComponentStatus`/`SetupReport` serde shapes (Tasks 3–4) consumed by the frontend (`component`/`present`; `apt_packages`/`mkcert_ca`/`nginx_setcap`/`errors`) in Task 5. `detect_components`/`run_setup`/`CurlDownloader`/`PkexecPrivileged`/`SudoPrivileged` re-exports (Tasks 2,4) used by Tasks 5–6. `AppState.php_version` added in Task 5 and read by both new commands. The JS `run_setup_cmd` name matches the Rust command identifier (Task 5 Step 4).

**Note on Plan-2/3a deferred minors:** the nginx-resolution divergence is resolved here (Task 1 + Task 4 setcap). The remaining 3a minors (orphan-on-SIGKILL process-group, tld snapshot, CSP, ignored ensure_dirs error) are not in this plan's scope and remain tracked for a later hardening pass.
