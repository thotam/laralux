# No-apt Spec 1 (nginx + redis/Valkey + mkcert) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Install nginx, redis (Valkey), and mkcert as downloaded binaries into the versioned layout (`bin/<tool>/<version>/` + `current`), drop them from apt, and fix detection — so only MariaDB remains on apt (Spec 2).

**Architecture:** Three new `core` modules (`mkcert_static`, `nginx_static`, `redis_static`) mirroring `coredns.rs`/`php_static.rs`: download → (extract) → atomic temp→rename into `version_dir` → `layout::set_current` → return version. `run_setup` installs the missing ones (best-effort, records `config.versions`), `apt_packages_for` empties them, `detect()` resolves via `managed_bin_dirs`, and `install_mkcert_ca` takes the downloaded binary path.

**Tech Stack:** Rust (laragon-core, zero Tauri deps), serde_json, mkcert/nginx/Valkey static binaries.

## Global Constraints

- `core` keeps **zero Tauri deps**. No `Co-Authored-By` trailer. TDD: failing test first for pure functions.
- Layout: install into `paths.version_dir(tool, ver)`, set `layout::set_current(paths, tool, ver)`, return the version string; idempotent on a non-empty existing binary; atomic temp→rename (copy+remove cross-device fallback); chmod 0755.
- Every install fn takes a trailing `sink: &dyn crate::progress::ProgressSink`; the meaningful download uses `downloader.fetch_with_progress(url, dest, sink)`.
- Sources/versions (verbatim): mkcert `https://github.com/FiloSottile/mkcert/releases/download/v1.4.4/mkcert-v1.4.4-linux-<amd64|arm64>`, `MKCERT_VERSION="1.4.4"`. nginx index `https://jirutka.github.io/nginx-binaries/index.json`, base `https://jirutka.github.io/nginx-binaries`, filename `nginx-<ver>-<x86_64|aarch64>-linux`. Valkey `https://download.valkey.io/releases/valkey-<ver>-jammy-<x86_64|arm64>.tar.gz`, `VALKEY_VERSION="9.1.0"`, tar contains `valkey-<ver>-jammy-<arch>/bin/{valkey-server,valkey-cli}`.
- Tool dir names: `mkcert` (bin `mkcert`), `nginx` (bin `nginx`), `redis` (bins `redis-server`,`redis-cli`).
- `cargo test -p laragon-core`; `cargo build -p laragon-desktop && cargo build -p laragonctl`. cargo fallback `$HOME/.cargo/bin/cargo`.

Patterns to copy: `core/src/coredns.rs` `ensure_coredns` (download+tar extract+atomic rename+set_current, `coredns_installed`), and `core/src/php_static.rs` `latest_patch`/JSON parse + `download_static_php` (single-binary install + set_current). Reuse their idioms.

---

### Task 1: `mkcert_static` module + `install_mkcert_ca(path)`

**Files:** Create `core/src/mkcert_static.rs`; modify `core/src/privileged.rs`, `core/src/setup.rs` (the mkcert-CA call site only), `core/src/lib.rs`.

**Interfaces produced:** `mkcert_static::{MKCERT_VERSION, mkcert_arch, mkcert_url, install_mkcert, MkcertError}`; `Privileged::install_mkcert_ca(&self, mkcert_bin: &Path)`.

- [ ] **Step 1: Write the failing tests** — create `core/src/mkcert_static.rs`:

```rust
use crate::paths::LaragonPaths;
use crate::progress::ProgressSink;
use crate::setup::Downloader;

pub const MKCERT_VERSION: &str = "1.4.4";

#[derive(Debug, thiserror::Error)]
pub enum MkcertError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn mkcert_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("amd64"), "aarch64" => Some("arm64"), _ => None }
}

pub fn mkcert_url(version: &str, arch: &str) -> String {
    format!("https://github.com/FiloSottile/mkcert/releases/download/v{version}/mkcert-v{version}-linux-{arch}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_and_arch() {
        assert_eq!(mkcert_url("1.4.4", "amd64"),
            "https://github.com/FiloSottile/mkcert/releases/download/v1.4.4/mkcert-v1.4.4-linux-amd64");
        assert_eq!(mkcert_arch(), match std::env::consts::ARCH { "x86_64" => Some("amd64"), "aarch64" => Some("arm64"), _ => None });
    }
}
```

- [ ] **Step 2: Run** — `cargo test -p laragon-core mkcert_static` → FAIL (some items missing) — actually it compiles the pure fns; ensure the test passes the URL assertion. If it already passes, proceed.

- [ ] **Step 3: Implement `install_mkcert`** — add above the tests:

```rust
/// True only if a non-empty regular file exists at `dest`.
fn installed(dest: &std::path::Path) -> bool {
    std::fs::metadata(dest).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
}

/// Download the static mkcert binary into bin/mkcert/<version>/mkcert (no apt).
pub fn install_mkcert(
    paths: &LaragonPaths, downloader: &dyn Downloader, sink: &dyn ProgressSink,
) -> Result<String, MkcertError> {
    let dir = paths.version_dir("mkcert", MKCERT_VERSION);
    let dest = dir.join("mkcert");
    if installed(&dest) {
        let _ = crate::layout::set_current(paths, "mkcert", MKCERT_VERSION);
        return Ok(MKCERT_VERSION.to_string());
    }
    let arch = mkcert_arch().ok_or_else(|| MkcertError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    std::fs::create_dir_all(&dir)?;
    let tmp = paths.tmp().join("mkcert.download");
    let _ = std::fs::remove_file(&tmp);
    downloader.fetch_with_progress(&mkcert_url(MKCERT_VERSION, arch), &tmp, sink)
        .map_err(|e| MkcertError::Download(e.to_string()))?;
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?; }
    std::fs::rename(&tmp, &dest).or_else(|_| {
        std::fs::copy(&tmp, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(&tmp))
    })?;
    crate::layout::set_current(paths, "mkcert", MKCERT_VERSION)?;
    Ok(MKCERT_VERSION.to_string())
}
```

- [ ] **Step 4: Change `install_mkcert_ca` to take the binary path.** In `core/src/privileged.rs`:
  - Trait: `fn install_mkcert_ca(&self, mkcert_bin: &Path) -> Result<(), PrivError>;`
  - `SudoPrivileged` + `PkexecPrivileged` impls: `run_escalated(&mkcert_bin.display().to_string(), &["-install".to_string()])` (run the resolved binary directly, un-escalated, exactly as the old `run_escalated("mkcert", &["-install"])` did — i.e. the "escalator" position holds the program; keep that pattern but use `mkcert_bin`). `FakePrivileged`: add a field `mkcert_ca_path: Arc<Mutex<Option<PathBuf>>>` + accessor `mkcert_ca_path()`, record the path, return Ok. Update the existing `installed_ca()` accessor/test if it asserted a bool — keep `installed_ca()` returning whether the path was recorded (`self.mkcert_ca_path.lock().unwrap().is_some()`).
  - Update the existing `install_mkcert_ca` test (FakePrivileged) to pass a `Path` and assert the recorded path.

- [ ] **Step 5: Fix the `run_setup` mkcert-CA caller (minimal, keep green).** In `core/src/setup.rs`, the existing `privileged.install_mkcert_ca()` call → resolve the binary first:
```rust
    match crate::bin::resolve_bin("mkcert", &crate::layout::managed_bin_dirs(paths)) {
        Some(mk) => match privileged.install_mkcert_ca(&mk) {
            Ok(()) => report.mkcert_ca = true,
            Err(e) => report.errors.push(format!("mkcert -install: {e}")),
        },
        None => report.errors.push("mkcert -install: mkcert not found".to_string()),
    }
```

- [ ] **Step 6: Re-export** — `core/src/lib.rs`: `pub mod mkcert_static;` + `pub use mkcert_static::{install_mkcert, MkcertError};`.

- [ ] **Step 7: Run** — `cargo test -p laragon-core` PASS; `cargo build -p laragon-desktop && cargo build -p laragonctl` clean.

- [ ] **Step 8: Commit**

```bash
git add core/src/mkcert_static.rs core/src/privileged.rs core/src/setup.rs core/src/lib.rs
git commit -m "feat(core): mkcert static binary into bin/mkcert/<ver> + install_mkcert_ca(path)"
```

---

### Task 2: `nginx_static` module

**Files:** Create `core/src/nginx_static.rs`; modify `core/src/lib.rs`.

**Interfaces produced:** `nginx_static::{NGINX_INDEX_URL, NGINX_BASE_URL, nginx_arch, latest_nginx, install_nginx, NginxError}`.

- [ ] **Step 1: Write the failing tests** — create `core/src/nginx_static.rs` with the pure parser + test:

```rust
use crate::paths::LaragonPaths;
use crate::progress::ProgressSink;
use crate::setup::Downloader;

pub const NGINX_INDEX_URL: &str = "https://jirutka.github.io/nginx-binaries/index.json";
pub const NGINX_BASE_URL: &str = "https://jirutka.github.io/nginx-binaries";

#[derive(Debug, thiserror::Error)]
pub enum NginxError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("no nginx build for this platform")]
    NoBuild,
    #[error("download failed: {0}")]
    Download(String),
    #[error("parse failed: {0}")]
    Parse(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn nginx_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("aarch64"), _ => None }
}

fn vkey(v: &str) -> Vec<u32> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() }

/// Highest linux/<arch> nginx entry in the index → (version, filename). Tolerant of malformed entries.
pub fn latest_nginx(arch: &str, index_json: &str) -> Option<(String, String)> {
    let arr: Vec<serde_json::Value> = serde_json::from_str(index_json).ok()?;
    let mut best: Option<(Vec<u32>, String, String)> = None;
    for e in &arr {
        let name = e.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let os = e.get("os").and_then(|v| v.as_str()).unwrap_or("");
        let a = e.get("arch").and_then(|v| v.as_str()).unwrap_or("");
        if name != "nginx" || os != "linux" || a != arch { continue; }
        let ver = match e.get("version").and_then(|v| v.as_str()) { Some(v) => v, None => continue };
        let file = match e.get("filename").and_then(|v| v.as_str()) { Some(f) => f, None => continue };
        let key = vkey(ver);
        if best.as_ref().map_or(true, |(bk, _, _)| &key > bk) {
            best = Some((key, ver.to_string(), file.to_string()));
        }
    }
    best.map(|(_, v, f)| (v, f))
}

#[cfg(test)]
mod tests {
    use super::*;
    const IDX: &str = r#"[
      {"name":"nginx","version":"1.24.0","arch":"x86_64","os":"linux","filename":"nginx-1.24.0-x86_64-linux"},
      {"name":"nginx","version":"1.31.2","arch":"x86_64","os":"linux","filename":"nginx-1.31.2-x86_64-linux"},
      {"name":"nginx","version":"1.31.2","arch":"aarch64","os":"linux","filename":"nginx-1.31.2-aarch64-linux"},
      {"name":"nginx","version":"1.9.0","arch":"x86_64","os":"linux","filename":"nginx-1.9.0-x86_64-linux"},
      {"name":"njs","version":"9.9.9","arch":"x86_64","os":"linux","filename":"x"}
    ]"#;
    #[test]
    fn picks_highest_linux_x86_64() {
        let (v, f) = latest_nginx("x86_64", IDX).unwrap();
        assert_eq!(v, "1.31.2"); // numeric: 1.31.2 > 1.24.0 > 1.9.0
        assert_eq!(f, "nginx-1.31.2-x86_64-linux");
        assert!(latest_nginx("riscv64", IDX).is_none());
    }
    #[test]
    fn arch_maps() {
        assert_eq!(nginx_arch(), match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("aarch64"), _ => None });
    }
}
```

- [ ] **Step 2: Run** — `cargo test -p laragon-core nginx_static` → PASS for the pure fns (write impl if the module doesn't compile).

- [ ] **Step 3: Implement `install_nginx`** — add above tests:

```rust
fn installed(dest: &std::path::Path) -> bool {
    std::fs::metadata(dest).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
}

/// Download the static nginx binary (latest from the index) into bin/nginx/<ver>/nginx.
pub fn install_nginx(
    paths: &LaragonPaths, downloader: &dyn Downloader, sink: &dyn ProgressSink,
) -> Result<String, NginxError> {
    let arch = nginx_arch().ok_or_else(|| NginxError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    let idx = paths.tmp().join("nginx-index.json");
    downloader.fetch(NGINX_INDEX_URL, &idx).map_err(|e| NginxError::Download(e.to_string()))?;
    let json = std::fs::read_to_string(&idx)?;
    let (ver, filename) = latest_nginx(arch, &json).ok_or(NginxError::NoBuild)?;
    let dir = paths.version_dir("nginx", &ver);
    let dest = dir.join("nginx");
    if installed(&dest) {
        let _ = crate::layout::set_current(paths, "nginx", &ver);
        return Ok(ver);
    }
    std::fs::create_dir_all(&dir)?;
    let tmp = paths.tmp().join("nginx.download");
    let _ = std::fs::remove_file(&tmp);
    downloader.fetch_with_progress(&format!("{NGINX_BASE_URL}/{filename}"), &tmp, sink)
        .map_err(|e| NginxError::Download(e.to_string()))?;
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?; }
    std::fs::rename(&tmp, &dest).or_else(|_| {
        std::fs::copy(&tmp, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(&tmp))
    })?;
    crate::layout::set_current(paths, "nginx", &ver)?;
    Ok(ver)
}
```
(Note: the `Parse` variant is reserved for future strictness; `latest_nginx` returning `None` maps to `NoBuild`. Leaving `Parse` unused is acceptable — or drop it; keep `NoBuild`/`Download`/`Arch`/`Io`. If the unused variant warns, remove `Parse`.)

- [ ] **Step 4: Re-export** — `core/src/lib.rs`: `pub mod nginx_static;` + `pub use nginx_static::{install_nginx, NginxError};`.

- [ ] **Step 5: Run + Commit** — `cargo test -p laragon-core` PASS; build clean.

```bash
git add core/src/nginx_static.rs core/src/lib.rs
git commit -m "feat(core): nginx static binary (jirutka index) into bin/nginx/<ver>"
```

---

### Task 3: `redis_static` module (Valkey)

**Files:** Create `core/src/redis_static.rs`; modify `core/src/lib.rs`.

**Interfaces produced:** `redis_static::{VALKEY_VERSION, valkey_arch, valkey_url, install_redis, RedisError}`.

- [ ] **Step 1: Write the failing tests** — create `core/src/redis_static.rs`:

```rust
use crate::paths::LaragonPaths;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

pub const VALKEY_VERSION: &str = "9.1.0";

#[derive(Debug, thiserror::Error)]
pub enum RedisError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn valkey_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("arm64"), _ => None }
}

pub fn valkey_url(version: &str, arch: &str) -> String {
    format!("https://download.valkey.io/releases/valkey-{version}-jammy-{arch}.tar.gz")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_and_arch() {
        assert_eq!(valkey_url("9.1.0", "x86_64"),
            "https://download.valkey.io/releases/valkey-9.1.0-jammy-x86_64.tar.gz");
        assert_eq!(valkey_arch(), match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("arm64"), _ => None });
    }
}
```

- [ ] **Step 2: Implement `install_redis`** — extract the tarball into a temp dir, find `valkey-server`/`valkey-cli`, install them as `redis-server`/`redis-cli`:

```rust
fn installed(dest: &std::path::Path) -> bool {
    std::fs::metadata(dest).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
}

/// Find a file named `name` anywhere under `root` (shallow walk of dirs).
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

/// Download the Valkey tarball and install valkey-server/valkey-cli as
/// bin/redis/<ver>/{redis-server,redis-cli} (drop-in for redis).
pub fn install_redis(
    paths: &LaragonPaths, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, RedisError> {
    let dir = paths.version_dir("redis", VALKEY_VERSION);
    let server = dir.join("redis-server");
    if installed(&server) {
        let _ = crate::layout::set_current(paths, "redis", VALKEY_VERSION);
        return Ok(VALKEY_VERSION.to_string());
    }
    let arch = valkey_arch().ok_or_else(|| RedisError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    std::fs::create_dir_all(&dir)?;
    let tgz = paths.tmp().join("valkey.tar.gz");
    downloader.fetch_with_progress(&valkey_url(VALKEY_VERSION, arch), &tgz, sink)
        .map_err(|e| RedisError::Download(e.to_string()))?;
    let xdir = paths.tmp().join("valkey-extract");
    let _ = std::fs::remove_dir_all(&xdir);
    std::fs::create_dir_all(&xdir)?;
    runner.run("tar", &["-xzf".into(), tgz.display().to_string(), "-C".into(), xdir.display().to_string()], None)
        .map_err(|e| RedisError::Extract(e.to_string()))?;
    let vs = find_under(&xdir, "valkey-server").ok_or_else(|| RedisError::Extract("valkey-server not found in archive".into()))?;
    install_one(&vs, &server)?;
    if let Some(vc) = find_under(&xdir, "valkey-cli") { let _ = install_one(&vc, &dir.join("redis-cli")); }
    crate::layout::set_current(paths, "redis", VALKEY_VERSION)?;
    Ok(VALKEY_VERSION.to_string())
}

fn install_one(src: &std::path::Path, dest: &std::path::Path) -> Result<(), RedisError> {
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(src, std::fs::Permissions::from_mode(0o755)); }
    std::fs::rename(src, dest).or_else(|_| {
        std::fs::copy(src, dest).map(|_| ()).and_then(|_| std::fs::remove_file(src))
    })?;
    Ok(())
}
```

- [ ] **Step 3: Re-export** — `core/src/lib.rs`: `pub mod redis_static;` + `pub use redis_static::{install_redis, RedisError};`.

- [ ] **Step 4: Run + Commit** — `cargo test -p laragon-core` PASS; build clean.

```bash
git add core/src/redis_static.rs core/src/lib.rs
git commit -m "feat(core): redis (Valkey) static tarball into bin/redis/<ver>"
```

---

### Task 4: wire into `run_setup`, drop apt, fix `detect`

**Files:** Modify `core/src/setup.rs`.

**Interfaces produced:** `apt_packages_for(Nginx|Redis|Mkcert)` empty; `SetupReport` gains `nginx_fetched`/`redis_fetched`/`mkcert_fetched`; `run_setup` installs the three; `detect` uses `managed_bin_dirs`.

- [ ] **Step 1: Empty the apt packages.** In `apt_packages_for`, change `Component::Nginx`, `Component::Redis`, `Component::Mkcert` to return `Vec::new()`. Leave `Component::Mariadb => vec!["mariadb-server".to_string()]`. Update the existing tests: assert `apt_packages_for(Nginx|Redis|Mkcert)` is empty and `Mariadb` still has its package (adjust `mkcert_includes_nss_tools` → assert empty now, rename if appropriate).

- [ ] **Step 2: `detect` via managed_bin_dirs.** Change the `other` arm of `detect()` from `resolve_bin(&name, &[paths.bin()])` to `crate::bin::resolve_bin(&name, &crate::layout::managed_bin_dirs(paths))`. (resolve_bin keeps its `$PATH` fallback, so apt mariadb still resolves until Spec 2; the new layout tools resolve under `bin/*/current`.)

- [ ] **Step 3: `SetupReport` fields.** Add `pub nginx_fetched: bool,`, `pub redis_fetched: bool,`, `pub mkcert_fetched: bool,` to `SetupReport` and initialize them `false` in the `run_setup` literal (mirror `mailpit_fetched`/`composer_fetched`).

- [ ] **Step 4: Install blocks in `run_setup`.** After the existing composer block (and before/after mailpit — anywhere after PHP), add (best-effort, recording the version into config):

```rust
    if missing.contains(&Component::Mkcert) {
        match crate::mkcert_static::install_mkcert(paths, downloader, sink) {
            Ok(ver) => { report.mkcert_fetched = true; record_version(paths, "mkcert", &ver); }
            Err(e) => report.errors.push(format!("install mkcert: {e}")),
        }
    }
    if missing.contains(&Component::Nginx) {
        match crate::nginx_static::install_nginx(paths, downloader, sink) {
            Ok(ver) => { report.nginx_fetched = true; record_version(paths, "nginx", &ver); }
            Err(e) => report.errors.push(format!("install nginx: {e}")),
        }
    }
    if missing.contains(&Component::Redis) {
        match crate::redis_static::install_redis(paths, downloader, runner, sink) {
            Ok(ver) => { report.redis_fetched = true; record_version(paths, "redis", &ver); }
            Err(e) => report.errors.push(format!("install redis: {e}")),
        }
    }
```
Add a small helper near `run_setup` (or inline the load/insert/save) — if a `record_version`-style helper doesn't already exist, write one:
```rust
fn record_version(paths: &LaragonPaths, tool: &str, version: &str) {
    let mut cfg = crate::config::Config::load(&paths.config_file()).unwrap_or_default();
    cfg.versions.insert(tool.to_string(), version.to_string());
    let _ = cfg.save(&paths.config_file());
}
```
(If the PHP/mailpit blocks already write `config.versions` inline, follow the same inline style instead of a helper — match the existing code. The final `apply_versions(paths, &Config::load(...))` at the end of `run_setup` materializes all `current` symlinks.)

- [ ] **Step 5: Run** — `cargo test -p laragon-core` PASS (update any `run_setup`/`detect`/`apt_packages_for` tests that assumed apt for these three); `cargo build -p laragon-desktop && cargo build -p laragonctl` clean.

- [ ] **Step 6: Manual verification (live)** — with nginx/redis/mkcert removed from the system, run the desktop app's Setup: confirm `bin/nginx/<ver>/nginx`, `bin/redis/<ver>/{redis-server,redis-cli}`, `bin/mkcert/<ver>/mkcert` appear with `current` symlinks, `config.versions` has nginx/redis/mkcert, the components show Installed, the stack starts (nginx binds, redis runs), and `apt packages: 0 installed` in the report.

- [ ] **Step 7: Commit**

```bash
git add core/src/setup.rs
git commit -m "feat(core): run_setup downloads nginx/redis/mkcert into layout; drop them from apt"
```

---

## Self-Review

**1. Spec coverage:** mkcert_static + install_mkcert_ca(path) (T1, §3.1/3.4); nginx_static (T2, §3.2); redis_static/Valkey (T3, §3.3); run_setup wiring + apt empty + detect managed_bin_dirs + SetupReport (T4, §3.5); lib re-exports across tasks (§3.6). MariaDB stays apt (§6, Spec 2) — correct. mkcert CA uses the resolved path (T1 Step 5).

**2. Placeholder scan:** No TBD; versions/URLs are concrete consts. The nginx `Parse` variant is noted as optionally removable to avoid an unused-variant warning.

**3. Type consistency:** `install_mkcert`/`install_nginx`/`install_redis` all return `Result<String, _>` consumed by `run_setup` (T4); each takes `sink: &dyn ProgressSink` (matches `run_setup`'s `sink`); `install_redis` also takes `runner: &dyn CommandRunner` (run_setup has `runner`); `install_mkcert_ca(&Path)` (T1) is called with `resolve_bin("mkcert", managed_bin_dirs)` (T1 Step 5); `layout::set_current`/`version_dir`/`managed_bin_dirs` and `config.versions` match Spec 0. `detect` `other` arm uses `managed_bin_dirs` (T4) consistent with the PHP/Composer arms.
