# No-apt Spec 2 (MariaDB) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Install MariaDB from the official binary tarball into `bin/mariadb/<version>/` (a full basedir + top-level symlinks), drop it from apt (the last package), and remove the dead apt/apparmor machinery — stack fully no-apt.

**Architecture:** New `mariadb_static` module extracts the tarball into the version dir (basedir) and symlinks `mariadbd`/`mariadb-install-db`/`mariadb` at the top so the layout resolver finds them; `MariadbService` passes `--basedir` and inits via the bundled `mariadb-install-db`; `run_setup` installs it best-effort + records `config.versions`; then the now-unused `apt_install`/`add_apt_repository`/`allow_mariadb_apparmor` are removed.

**Tech Stack:** Rust (laralux-core, zero Tauri deps), MariaDB 11.4 LTS binary tarball, tar.

## Global Constraints

- `core` keeps **zero Tauri deps**. No `Co-Authored-By` trailer. TDD for pure functions.
- Source: `https://archive.mariadb.org/mariadb-<ver>/bintar-linux-systemd-<arch>/mariadb-<ver>-linux-systemd-<arch>.tar.gz`, `MARIADB_VERSION="11.4.12"`, arch `x86_64`/`aarch64`. The tarball extracts to a single top dir `mariadb-<ver>-linux-systemd-<arch>/` containing `bin/`, `lib/`, `share/`, `scripts/`. ~360 MB.
- Layout: extract the whole tree into `paths.version_dir("mariadb", ver)` (= basedir); top-level RELATIVE symlinks `mariadbd`→`bin/mariadbd`, `mariadb-install-db`→(`bin/mariadb-install-db` or `scripts/mariadb-install-db`), `mariadb`→`bin/mariadb`; `layout::set_current(paths,"mariadb",ver)`; idempotent on an existing `bin/mariadb/<ver>/mariadbd`.
- `report.apt_packages` (a `Vec<String>`) MUST stay (laraluxctl + frontend read it) — set it `Vec::new()`.
- `cargo test -p laralux-core`; `cargo build -p laralux-desktop && cargo build -p laraluxctl`. cargo fallback `$HOME/.cargo/bin/cargo`.

Pattern to copy: `core/src/redis_static.rs` (tarball download + tar extract + `find_under` walk + atomic install + set_current) and `core/src/coredns.rs`.

---

### Task 1: `mariadb_static` module

**Files:** Create `core/src/mariadb_static.rs`; modify `core/src/lib.rs`.

**Interfaces produced:** `mariadb_static::{MARIADB_VERSION, mariadb_arch, mariadb_url, install_mariadb, MariadbError}`.

- [ ] **Step 1: Write the failing tests** — create `core/src/mariadb_static.rs`:

```rust
use crate::paths::LaraluxPaths;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

pub const MARIADB_VERSION: &str = "11.4.12";

#[derive(Debug, thiserror::Error)]
pub enum MariadbError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("layout error: {0}")]
    Layout(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn mariadb_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("aarch64"), _ => None }
}

pub fn mariadb_url(version: &str, arch: &str) -> String {
    format!("https://archive.mariadb.org/mariadb-{version}/bintar-linux-systemd-{arch}/mariadb-{version}-linux-systemd-{arch}.tar.gz")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_and_arch() {
        assert_eq!(mariadb_url("11.4.12", "x86_64"),
            "https://archive.mariadb.org/mariadb-11.4.12/bintar-linux-systemd-x86_64/mariadb-11.4.12-linux-systemd-x86_64.tar.gz");
        assert_eq!(mariadb_arch(), match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("aarch64"), _ => None });
    }
}
```

- [ ] **Step 2: Run** — `cargo test -p laralux-core mariadb_static` → PASS for the pure fns.

- [ ] **Step 3: Implement `install_mariadb`** — add above the tests:

```rust
/// Find a file/symlink-target named `name` anywhere under `root` (DFS); returns its path.
fn find_under(root: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() { stack.push(p); }
                else if p.file_name().map(|n| n == name).unwrap_or(false) { return Some(p); }
            }
        }
    }
    None
}

/// Make `link` (under basedir) a relative symlink to `target_rel` (a path relative to basedir).
fn rel_symlink(basedir: &std::path::Path, link_name: &str, target_rel: &str) -> std::io::Result<()> {
    let link = basedir.join(link_name);
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    { std::os::unix::fs::symlink(target_rel, &link)?; }
    Ok(())
}

/// Download + extract the MariaDB binary tarball into bin/mariadb/<ver>/ (the basedir)
/// with top-level mariadbd/mariadb-install-db/mariadb symlinks for the resolver.
pub fn install_mariadb(
    paths: &LaraluxPaths, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, MariadbError> {
    let basedir = paths.version_dir("mariadb", MARIADB_VERSION);
    if basedir.join("mariadbd").exists() {
        let _ = crate::layout::set_current(paths, "mariadb", MARIADB_VERSION);
        return Ok(MARIADB_VERSION.to_string());
    }
    let arch = mariadb_arch().ok_or_else(|| MariadbError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    let tgz = paths.tmp().join("mariadb.tar.gz");
    downloader.fetch_with_progress(&mariadb_url(MARIADB_VERSION, arch), &tgz, sink)
        .map_err(|e| MariadbError::Download(e.to_string()))?;
    let xdir = paths.tmp().join("mariadb-extract");
    let _ = std::fs::remove_dir_all(&xdir);
    std::fs::create_dir_all(&xdir)?;
    runner.run("tar", &["-xzf".into(), tgz.display().to_string(), "-C".into(), xdir.display().to_string()], None)
        .map_err(|e| MariadbError::Extract(e.to_string()))?;
    // The tarball nests under a single dir `mariadb-<ver>-...`; move it to basedir.
    let top = std::fs::read_dir(&xdir)?.flatten()
        .map(|e| e.path()).find(|p| p.is_dir())
        .ok_or_else(|| MariadbError::Extract("empty archive".into()))?;
    let _ = std::fs::remove_dir_all(&basedir);
    std::fs::create_dir_all(basedir.parent().unwrap())?;
    std::fs::rename(&top, &basedir).or_else(|_| {
        // cross-device: recursive copy then remove (best-effort via `cp -a`)
        runner.run("cp", &["-a".into(), top.display().to_string(), basedir.display().to_string()], None)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
            .and_then(|_| std::fs::remove_dir_all(&top))
    })?;
    // Resolve the real binaries inside the basedir and symlink them at the top level.
    let mariadbd = find_under(&basedir, "mariadbd").ok_or_else(|| MariadbError::Layout("mariadbd not found".into()))?;
    let mariadbd_rel = mariadbd.strip_prefix(&basedir).map(|p| p.display().to_string()).unwrap_or_else(|_| "bin/mariadbd".into());
    rel_symlink(&basedir, "mariadbd", &mariadbd_rel)?;
    if let Some(idb) = find_under(&basedir, "mariadb-install-db") {
        let rel = idb.strip_prefix(&basedir).map(|p| p.display().to_string()).unwrap_or_else(|_| "scripts/mariadb-install-db".into());
        let _ = rel_symlink(&basedir, "mariadb-install-db", &rel);
    }
    if let Some(cli) = find_under(&basedir, "mariadb") {
        // skip if it's the basedir-relative "mariadb" we'd be creating; only the bin/ client
        if cli != basedir.join("mariadb") {
            let rel = cli.strip_prefix(&basedir).map(|p| p.display().to_string()).unwrap_or_else(|_| "bin/mariadb".into());
            let _ = rel_symlink(&basedir, "mariadb", &rel);
        }
    }
    crate::layout::set_current(paths, "mariadb", MARIADB_VERSION)?;
    Ok(MARIADB_VERSION.to_string())
}
```
(Note: `find_under(basedir, "mariadb")` for the client could match a directory or the dir name itself; the guard `cli != basedir.join("mariadb")` plus `find_under` only returning non-dir files keeps it to the real client binary. If this is fragile, prefer `basedir.join("bin").join("mariadb")` directly when it exists.)

- [ ] **Step 4: Re-export** — `core/src/lib.rs`: `pub mod mariadb_static;` + `pub use mariadb_static::{install_mariadb, MariadbError};`.

- [ ] **Step 5: Run + Commit** — `cargo test -p laralux-core` PASS; build clean.

```bash
git add core/src/mariadb_static.rs core/src/lib.rs
git commit -m "feat(core): MariaDB binary tarball into bin/mariadb/<ver> (basedir + symlinks)"
```

---

### Task 2: `MariadbService` — basedir + bundled install-db

**Files:** Modify `core/src/service/mariadb.rs`.

**Interfaces:** `command` adds `--basedir`; `init` uses the resolved `mariadb-install-db` + `--basedir`.

- [ ] **Step 1: Add basedir + update command/install_db_args.** In `core/src/service/mariadb.rs`:
  - Add: `fn basedir(&self, paths: &LaraluxPaths) -> std::path::PathBuf { paths.bin().join("mariadb").join("current") }`.
  - `command`:
```rust
    fn command(&self, paths: &LaraluxPaths) -> SpawnSpec {
        SpawnSpec::new("mariadbd")
            .arg(format!("--defaults-file={}", self.cnf_path(paths).display()))
            .arg(format!("--basedir={}", self.basedir(paths).display()))
    }
```
  - `install_db_args`: add a `--basedir` entry:
```rust
    fn install_db_args(&self, paths: &LaraluxPaths) -> Vec<String> {
        vec![
            "--no-defaults".to_string(),
            format!("--basedir={}", self.basedir(paths).display()),
            format!("--datadir={}", self.datadir(paths).display()),
            "--auth-root-authentication-method=normal".to_string(),
        ]
    }
```
  - `init`: resolve the bundled tool via the layout instead of `$PATH`:
```rust
    fn init(&self, paths: &LaraluxPaths) -> Result<(), ServiceError> {
        self.write_config(paths)?;
        let tool = crate::bin::resolve_bin("mariadb-install-db", &crate::layout::managed_bin_dirs(paths))
            .ok_or_else(|| ServiceError::Init("mariadb-install-db not found".into()))?;
        let status = std::process::Command::new(&tool)
            .args(self.install_db_args(paths))
            .status()
            .map_err(|e| ServiceError::Init(format!("mariadb-install-db: {e}")))?;
        if !status.success() {
            return Err(ServiceError::Init("mariadb-install-db failed".into()));
        }
        Ok(())
    }
```

- [ ] **Step 2: Update the unit tests** in the same file:
  - `command_and_kind`: still asserts program `mariadbd` and a `--defaults-file=` arg; add `assert!(spec.args.iter().any(|a| a.starts_with("--basedir=")));`.
  - `install_db_args_use_no_defaults_not_defaults_file`: add `assert!(args.iter().any(|a| a.starts_with("--basedir=")));` (keep the existing assertions).

- [ ] **Step 3: Run + Commit** — `cargo test -p laralux-core mariadb` PASS; `cargo build -p laralux-desktop && cargo build -p laraluxctl` clean.

```bash
git add core/src/service/mariadb.rs
git commit -m "feat(core): MariadbService passes --basedir; init via bundled mariadb-install-db"
```

---

### Task 3: wire into `run_setup`, drop MariaDB from apt, drop apparmor

**Files:** Modify `core/src/setup.rs`.

**Interfaces:** `apt_packages_for(Mariadb)` empty; `SetupReport.mariadb_fetched`; `run_setup` installs MariaDB, no longer calls `apt_install`/`allow_mariadb_apparmor`.

- [ ] **Step 1: Empty MariaDB's apt entry.** In `apt_packages_for`, change `Component::Mariadb => vec!["mariadb-server".to_string()]` to `Component::Mariadb => Vec::new()`. Update the existing `Mariadb` test (the one asserting `["mariadb-server"]`, ~line 470) to assert empty (rename e.g. `mariadb_has_no_apt_package`).

- [ ] **Step 2: `SetupReport.mariadb_fetched`.** Add `pub mariadb_fetched: bool,` to `SetupReport` and initialize `false` in the `run_setup` literal (mirror `mailpit_fetched`).

- [ ] **Step 3: Remove the apt_install call; keep `report.apt_packages` empty.** Replace the block:
```rust
    let apt_packages: Vec<String> =
        missing.iter().flat_map(|&c| apt_packages_for(c)).collect();
    report.apt_packages = apt_packages.clone();
    if !apt_packages.is_empty() {
        if let Err(e) = privileged.apt_install(&apt_packages) {
            report.errors.push(format!("apt_install: {e}"));
        }
    }
```
with:
```rust
    // All stack components are downloaded now — nothing is installed via apt.
    report.apt_packages = Vec::new();
```

- [ ] **Step 4: Add the MariaDB install block.** Alongside the nginx/redis/mkcert blocks (after them is fine):
```rust
    if missing.contains(&Component::Mariadb) {
        match crate::mariadb_static::install_mariadb(paths, downloader, runner, sink) {
            Ok(ver) => { report.mariadb_fetched = true; record_version(paths, "mariadb", &ver); }
            Err(e) => report.errors.push(format!("install mariadb: {e}")),
        }
    }
```
(Use the same `record_version` helper / inline style the nginx/redis/mkcert blocks use.)

- [ ] **Step 5: Remove the apparmor call.** Delete the block:
```rust
    if let Err(e) = privileged.allow_mariadb_apparmor() {
        report.errors.push(format!("mariadb apparmor: {e}"));
    }
```
(The tarball `mariadbd` lives under `~/laralux` — no Ubuntu AppArmor profile applies.) Remove the `run_setup_configures_mariadb_apparmor` test (it asserts the call happened). Keep `disable_system_services` + its test.

- [ ] **Step 6: Run + Commit** — `cargo test -p laralux-core` PASS (update any run_setup test that expected mariadb apt / apparmor); `cargo build -p laralux-desktop && cargo build -p laraluxctl` clean.

```bash
git add core/src/setup.rs
git commit -m "feat(core): run_setup downloads MariaDB into layout; drop it from apt + apparmor"
```

---

### Task 4: remove the dead apt/apparmor machinery

**Files:** Modify `core/src/privileged.rs` (and `core/src/setup.rs` if `apt_packages_for` is removed).

**Interfaces:** `Privileged` loses `apt_install`, `add_apt_repository`, `allow_mariadb_apparmor`.

- [ ] **Step 1: Confirm no callers remain.** `grep -rn 'apt_install\|add_apt_repository\|allow_mariadb_apparmor' core/src src-tauri/src laraluxctl/src` — after Task 3, the only matches should be the trait + impls + helpers + tests in `core/src/privileged.rs` (no call sites in `setup.rs`/desktop/cli). If any real caller remains, STOP and report.

- [ ] **Step 2: Remove from `core/src/privileged.rs`:**
  - Trait methods `apt_install`, `add_apt_repository`, `allow_mariadb_apparmor`.
  - Their impls in `SudoPrivileged`, `PkexecPrivileged`, `FakePrivileged`.
  - The free helpers `apt_argv`, `add_repo_argv`, `mariadb_apparmor_argv`.
  - `FakePrivileged` fields + accessors: `apt_installs`/`apt_installs()`, `add_repos`/`add_repos()`, the apparmor bool + `mariadb_apparmor_configured()`.
  - The related unit tests: `fake_records_apt_installs`, `add_repo_argv_builds_add_apt_repository`, `fake_records_add_repo` (if present), `mariadb_apparmor_argv_runs_parser_with_laralux_rule`, the apparmor fake test.
  - Keep everything else (`write_etc_hosts`, `install_mkcert_ca`, `setcap_nginx`, `disable_system_services`, `write_resolved_dropin`/`remove_resolved_dropin`, `run_escalated`).

- [ ] **Step 3: (optional) `apt_packages_for`.** It now returns empty for every component and is only referenced by tests + (no longer) run_setup. You MAY keep it + its tests as documentation, OR remove `apt_packages_for` and its tests. KEEP `report.apt_packages` (set to `Vec::new()` in Task 3) — laraluxctl/frontend read it. If you remove `apt_packages_for`, also remove the `apt_packages_for_*` tests. Either is fine; choose the one that leaves a clean build with no unused-fn warning.

- [ ] **Step 4: Run + Commit** — `cargo test -p laralux-core` PASS; `cargo build -p laralux-desktop && cargo build -p laraluxctl` clean (no dangling references, no unused warnings).

```bash
git add core/src/privileged.rs core/src/setup.rs
git commit -m "refactor(core): remove dead apt_install/add_apt_repository/allow_mariadb_apparmor (no-apt)"
```

---

## Self-Review

**1. Spec coverage:** mariadb_static module (T1, §3.1); MariadbService basedir+init (T2, §3.2); run_setup wiring + apt empty + apparmor removed + report field (T3, §3.3); remove dead apt/apparmor machinery (T4, §3.4); lib re-exports (T1, §3.5). `report.apt_packages` kept empty (laraluxctl/frontend). disable_system_services kept (best-effort). detect() already managed_bin_dirs (Spec 1).

**2. Placeholder scan:** No TBD; version/URL concrete. The `cp -a` cross-device fallback + `find_under`/`rel_symlink` are concrete. The "client `mariadb` symlink" edge is noted with a concrete safer alternative.

**3. Type consistency:** `install_mariadb(paths, downloader, runner, sink) -> Result<String,_>` consumed by run_setup (T3) with `record_version`; `MariadbService.basedir` used by `command`/`install_db_args`/`init` (T2); `mariadb_arch`/`mariadb_url` pure (T1). After T4, no symbol references `apt_install`/`add_apt_repository`/`allow_mariadb_apparmor`. `report.apt_packages` field retained.
