# Laragon Linux — No-apt Spec 2 (MariaDB) into the versioned layout

**Date:** 2026-06-26
**Status:** Design (goal-directed); the final no-apt step.
**Goal:** Install **MariaDB** from the official binary tarball into the versioned layout (`bin/mariadb/<version>/`), remove it from apt — the last apt package — and then remove the now-dead apt machinery (`apt_install`/`add_apt_repository`/`allow_mariadb_apparmor`) so the stack is fully no-apt.

---

## 1. Context & current state

After Spec 1, only **MariaDB** remains on apt (`apt_packages_for(Mariadb) == ["mariadb-server"]`). `MariadbService` (`core/src/service/mariadb.rs`): runs `mariadbd --defaults-file=<etc/mariadb/my.cnf>` (program `mariadbd`, resolved by the orchestrator via `managed_bin_dirs`); `init()` runs `mariadb-install-db --no-defaults --datadir=<data/mariadb> --auth-root-authentication-method=normal` (program `mariadb-install-db`, resolved from `$PATH` — apt provided it); `needs_init` = datadir/`mysql` dir absent; `my.cnf` sets datadir/socket/port/bind/pid/log-error. `run_setup` apt-installs `mariadb-server`, then `disable_system_services(["nginx","mariadb","redis-server"])` and `allow_mariadb_apparmor()` (Ubuntu's AppArmor profile confines the apt `mariadbd`). `Privileged` still has `apt_install`/`add_apt_repository`/`allow_mariadb_apparmor`.

Unlike nginx (1 binary) / redis (2 binaries), MariaDB needs its whole distribution (a **basedir** with `bin/`, `lib/plugin/`, `share/` error messages + bootstrap SQL, `scripts/mariadb-install-db`). So we extract the entire tarball into `bin/mariadb/<version>/` (the basedir) and expose `mariadbd`/`mariadb-install-db` (and the `mariadb` client) as top-level entries so the layout resolver finds them.

Verified source (June 2026): `https://archive.mariadb.org/mariadb-11.4.12/bintar-linux-systemd-x86_64/mariadb-11.4.12-linux-systemd-x86_64.tar.gz` (~360 MB; MariaDB **11.4 LTS**, glibc ≥2.19; the `systemd` bintar runs fine standalone in foreground). The tarball extracts to a single top dir `mariadb-11.4.12-linux-systemd-x86_64/` containing `bin/`, `lib/`, `share/`, `scripts/`.

## 2. Approach

Add `core/src/mariadb_static.rs`: download the tarball, extract the single top dir into `bin/mariadb/<version>/` (the basedir), create top-level symlinks (`mariadbd`, `mariadb-install-db`, `mariadb`) → their real locations inside the basedir, `set_current`. Adapt `MariadbService` to pass `--basedir=<bin/mariadb/current>` to both `mariadbd` and `mariadb-install-db`, and resolve `mariadb-install-db` via `managed_bin_dirs`. Wire into `run_setup`, empty MariaDB's apt entry, and remove the apt/apparmor machinery (the last consumers are gone). `core` stays Tauri-free; pure URL/version functions are unit-tested; the heavy download/extract is live-verified.

## 3. Architecture & components

### 3.1 `core/src/mariadb_static.rs` (new)
- `pub const MARIADB_VERSION: &str = "11.4.12";`
- `pub fn mariadb_arch() -> Option<&'static str>` — `x86_64→"x86_64"`, `aarch64→"aarch64"`, else `None`.
- `pub fn mariadb_url(version, arch) -> String` → `https://archive.mariadb.org/mariadb-{version}/bintar-linux-systemd-{arch}/mariadb-{version}-linux-systemd-{arch}.tar.gz`.
- `pub fn install_mariadb(paths, downloader, runner, sink) -> Result<String, MariadbError>`:
  - Idempotent: if `bin/mariadb/<ver>/mariadbd` exists (symlink/file) → `set_current` + return ver.
  - Map arch; `create_dir_all(tmp)`; download (`fetch_with_progress`) to `tmp/mariadb.tar.gz`; extract into a fresh `tmp/mariadb-extract/` (`tar -xzf … -C`); find the single sub-directory (`mariadb-<ver>-…`); move it to `bin/mariadb/<ver>/` (rename; if the dest exists remove it first); create relative top-level symlinks inside `bin/mariadb/<ver>/`:
    - `mariadbd` → the real `mariadbd` found under the basedir (`bin/mariadbd`),
    - `mariadb-install-db` → the real one (`bin/mariadb-install-db` if present, else `scripts/mariadb-install-db`),
    - `mariadb` → `bin/mariadb` (client) if present (best-effort).
    Use a directory-walk helper to locate each real binary; symlink targets are relative to the basedir (e.g. `bin/mariadbd`).
  - `layout::set_current(paths,"mariadb",MARIADB_VERSION)`; return the version.
- `pub enum MariadbError` (thiserror): `Arch(String)`, `Download(String)`, `Extract(String)`, `Layout(String)` (binary not found in archive), `Io(#[from] std::io::Error)`.

### 3.2 `MariadbService` adapts to the basedir (`core/src/service/mariadb.rs`)
- Add `fn basedir(&self, paths) -> PathBuf { paths.bin().join("mariadb").join("current") }`.
- `command`: `mariadbd --defaults-file=<cnf> --basedir=<basedir>` (the program `mariadbd` still resolves via `managed_bin_dirs` → `bin/mariadb/current/mariadbd`; `--basedir` makes plugin/share lookup explicit and robust to the symlinked path).
- `init`: resolve the install tool via `crate::bin::resolve_bin("mariadb-install-db", &crate::layout::managed_bin_dirs(paths))` (→ `bin/mariadb/current/mariadb-install-db`); run it with `--no-defaults --basedir=<basedir> --datadir=<datadir> --auth-root-authentication-method=normal`. (Add `--basedir` to `install_db_args`.) If the tool isn't resolvable → `ServiceError::Init("mariadb-install-db not found")`.
- `needs_init`/`health_check`/`pre_start`/`write_config` unchanged. The existing unit tests for `command`/`install_db_args` get the added `--basedir` (update assertions to also accept it; keep the `--defaults-file`/`--no-defaults` assertions).

### 3.3 `core/src/setup.rs` — wire in, drop apt, drop apparmor
- `apt_packages_for(Mariadb)` → `Vec::new()` (now ALL components are empty).
- `run_setup`: add a `Component::Mariadb` install block (best-effort, after the others): `mariadb_static::install_mariadb(paths, downloader, runner, sink)` → on `Ok(ver)` set `report.mariadb_fetched=true` + record `config.versions["mariadb"]`; on `Err` push to `report.errors`. Add `mariadb_fetched: bool` to `SetupReport`.
- Remove the apt block entirely (it would now always receive an empty package list): delete the `let apt_packages = …; if !apt_packages.is_empty() { privileged.apt_install(…) }` section, and `report.apt_packages` (or keep the field set to an empty vec for the UI's "apt packages: 0 installed" row — keep the field, set it `Vec::new()`, drop the install call). Keep the `apt_packages_for` function (still referenced by tests; or remove it + its tests — see §3.4).
- Remove the `allow_mariadb_apparmor()` call (the tarball `mariadbd` lives under `~/laragon`, outside Ubuntu's AppArmor profile for `/usr/sbin/mariadbd`).
- Keep `disable_system_services([...])` (best-effort: frees ports if a user has leftover distro units; it's `systemctl disable`, not apt).
- `detect()` already resolves the `other` arm (incl. Mariadb `mariadbd`) via `managed_bin_dirs` (Spec 1) with `$PATH` fallback — unchanged; the downloaded `mariadbd` is now found under `bin/mariadb/current`.

### 3.4 Remove the dead apt/apparmor machinery (`core/src/privileged.rs`, `core/src/setup.rs`)
With no caller left, remove from `Privileged`: `apt_install`, `add_apt_repository`, `allow_mariadb_apparmor` (trait + `SudoPrivileged`/`PkexecPrivileged`/`FakePrivileged` impls), the free helpers `apt_argv`/`add_repo_argv`/`mariadb_apparmor_argv`, the `FakePrivileged` fields/accessors (`apt_installs`/`add_repos`/`mariadb_apparmor_configured`), and their unit tests. In `setup.rs`, remove `apt_packages_for` + its tests and `stack_units_to_disable` only if `disable_system_services` is also dropped — but we KEEP `disable_system_services`, so keep `stack_units_to_disable`. (If removing `apt_packages_for` breaks the "apt packages: N installed" report row, keep `report.apt_packages = Vec::new()` and drop the function + its tests.)
- This is a focused cleanup task; do it last so the build stays green through the feature work.

### 3.5 `lib.rs` re-exports
Add `pub mod mariadb_static;` + `pub use mariadb_static::{install_mariadb, MariadbError};`.

## 4. Behavior & error handling
- **Self-contained:** after setup, `bin/mariadb/<ver>/` is a full MariaDB basedir with top-level `mariadbd`/`mariadb-install-db`/`mariadb` symlinks; `current` points at it; `config.versions["mariadb"]` records it. No apt anywhere in the stack.
- **Heavy download:** ~360 MB tarball — idempotent (skips when `bin/mariadb/<ver>/mariadbd` exists); best-effort (a failure records an error, doesn't abort other components); progress flows through `sink`.
- **datadir init:** first start (`needs_init`) runs the bundled `mariadb-install-db` with `--basedir`/`--datadir`; the existing `~/laragon/data/mariadb` datadir + `my.cnf` are reused.
- **No new root:** mariadb is a user-space download under `~/laragon`; no AppArmor profile applies (so `allow_mariadb_apparmor` is unnecessary and removed). The remaining privileged steps are unchanged (hosts, setcap nginx, resolved drop-in, mkcert CA).

## 5. Testing (TDD; no network/extract in unit tests)
- `mariadb_url`/`mariadb_arch` — exact string + arch map; `install_mariadb` is **live-verified** (360 MB download + extract + first-run datadir init + server start), like the other heavy installers; the pure fns carry the unit coverage.
- `MariadbService`: `command` includes `--basedir` and `--defaults-file`; `install_db_args` includes `--basedir`/`--datadir`/`--no-defaults`/auth method (update the existing assertions).
- `setup`: `apt_packages_for(Mariadb)` empty (or the function removed); `run_setup` no longer calls `apt_install`/`allow_mariadb_apparmor` (FakePrivileged assertions updated/removed); a Mariadb install attempt is made when missing.
- After the privileged-cleanup task: the workspace compiles with `apt_install`/`add_apt_repository`/`allow_mariadb_apparmor` gone (no dangling callers in core/desktop/cli).
- `cargo test -p laragon-core`; `cargo build -p laragon-desktop && cargo build -p laragonctl`.

## 6. Out of scope (backlog)
- A non-systemd MariaDB bintar fallback for systems without systemd/glibc-2.19 (the systemd variant runs standalone; YAGNI now).
- Trimming the 360 MB extract (the bintar includes test suites/headers); a future optimization could prune `mysql-test/`, `man/`, `include/` after extract.
- aarch64 verification (the URL maps it, but only x86_64 is the primary verified target).
- Pinning/override UI for versions.
