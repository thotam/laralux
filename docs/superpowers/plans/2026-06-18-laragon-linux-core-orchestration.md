# Laragon Linux — Plan 1: Core Orchestration (headless) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a GUI-independent Rust core that generates per-service config under `~/laragon/` and starts/stops the stack (nginx, php-fpm, mariadb, redis, mailpit), plus a `laragonctl` CLI to exercise it.

**Architecture:** A Cargo workspace with a pure-logic `core` lib crate and a thin `laragonctl` binary. All business logic (path layout, config generation, command building, lifecycle ordering) lives in `core` and is unit-tested without spawning real processes. Process spawning is hidden behind a `ProcessSpawner` trait so the orchestrator is tested with a fake spawner; a `RealSpawner` wraps `std::process::Command` for production. No Tauri, no GUI, no privileged operations in this plan.

**Tech Stack:** Rust (edition 2021), `serde` + `toml` for config, `thiserror` for errors, `std::process` / `std::net` for spawning and health probes. Build/test with `cargo`.

## Global Constraints

- Target OS: Ubuntu 26.04 (Linux, systemd) — but `core` itself must not depend on systemd; it spawns processes directly.
- Working directory layout root: `~/laragon/` (override-able by constructing `LaragonPaths` with a custom root — tests use a tempdir).
- Config file: `~/laragon/laragon.toml` (TOML). Replaces Windows `laragon.ini`.
- Default TLD: `dev`. Default PHP version string: `8.4`.
- `core` crate MUST have zero Tauri dependencies and MUST NOT touch `/etc` or require root in this plan.
- Service start order (dependencies): `mariadb`, `redis` → `php-fpm` → `nginx`. `mailpit` independent.
- Follow TDD: write the failing test first, watch it fail, implement minimally, watch it pass, commit.

---

### Task 1: Workspace scaffold

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `core/Cargo.toml`
- Create: `core/src/lib.rs`
- Create: `laragonctl/Cargo.toml`
- Create: `laragonctl/src/main.rs`
- Create: `.gitignore`

- [ ] **Step 1: Create workspace root `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = ["core", "laragonctl"]

[workspace.package]
edition = "2021"
version = "0.1.0"
license = "MIT"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
toml = "0.8"
thiserror = "1"
```

- [ ] **Step 2: Create `core/Cargo.toml`**

```toml
[package]
name = "laragon-core"
edition.workspace = true
version.workspace = true
license.workspace = true

[lib]
name = "laragon_core"
path = "src/lib.rs"

[dependencies]
serde.workspace = true
toml.workspace = true
thiserror.workspace = true
```

- [ ] **Step 3: Create `core/src/lib.rs`**

```rust
//! Laragon Linux core: GUI-independent service orchestration.

pub mod paths;
```

(Other modules are added by later tasks. Keep this file as the module index.)

- [ ] **Step 4: Create `laragonctl/Cargo.toml`**

```toml
[package]
name = "laragonctl"
edition.workspace = true
version.workspace = true
license.workspace = true

[[bin]]
name = "laragonctl"
path = "src/main.rs"

[dependencies]
laragon-core = { path = "../core" }
```

- [ ] **Step 5: Create `laragonctl/src/main.rs`**

```rust
fn main() {
    println!("laragonctl 0.1.0");
}
```

- [ ] **Step 6: Create `.gitignore`**

```gitignore
/target
**/*.rs.bk
Cargo.lock
```

- [ ] **Step 7: Verify the workspace builds and tests run**

Run: `cargo test`
Expected: PASS — compiles both crates, `running 0 tests` for `laragon-core`.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml core laragonctl .gitignore
git commit -m "chore: scaffold cargo workspace (core + laragonctl)"
```

---

### Task 2: LaragonPaths — directory layout

**Files:**
- Create: `core/src/paths.rs`
- Modify: `core/src/lib.rs` (already declares `pub mod paths;`)

**Interfaces:**
- Produces:
  - `struct LaragonPaths { root: PathBuf }`
  - `LaragonPaths::new(root: PathBuf) -> LaragonPaths`
  - `LaragonPaths::default_root() -> PathBuf` (returns `$HOME/laragon`)
  - `fn root(&self) -> &Path`, `www`, `etc`, `data`, `log`, `tmp`, `ssl` → each `-> PathBuf`
  - `fn etc_for(&self, sub: &str) -> PathBuf` (e.g. `etc/nginx`)
  - `fn config_file(&self) -> PathBuf` (`<root>/laragon.toml`)
  - `fn ensure_dirs(&self) -> std::io::Result<()>` (creates www/etc/data/log/tmp/ssl)

- [ ] **Step 1: Write the failing test**

Append to `core/src/paths.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_subpaths_under_root() {
        let p = LaragonPaths::new("/tmp/lara".into());
        assert_eq!(p.root(), std::path::Path::new("/tmp/lara"));
        assert_eq!(p.www(), std::path::Path::new("/tmp/lara/www"));
        assert_eq!(p.etc(), std::path::Path::new("/tmp/lara/etc"));
        assert_eq!(p.etc_for("nginx"), std::path::Path::new("/tmp/lara/etc/nginx"));
        assert_eq!(p.config_file(), std::path::Path::new("/tmp/lara/laragon.toml"));
    }

    #[test]
    fn ensure_dirs_creates_layout() {
        let tmp = std::env::temp_dir().join(format!("lara-test-{}", std::process::id()));
        let p = LaragonPaths::new(tmp.clone());
        p.ensure_dirs().unwrap();
        for sub in ["www", "etc", "data", "log", "tmp", "ssl"] {
            assert!(tmp.join(sub).is_dir(), "missing {sub}");
        }
        std::fs::remove_dir_all(&tmp).ok();
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core paths`
Expected: FAIL — `cannot find type LaragonPaths`.

- [ ] **Step 3: Write minimal implementation**

Prepend to `core/src/paths.rs` (above the test module):

```rust
use std::path::{Path, PathBuf};

/// Resolves the `~/laragon/` directory layout.
#[derive(Clone, Debug)]
pub struct LaragonPaths {
    root: PathBuf,
}

impl LaragonPaths {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// `$HOME/laragon`, falling back to `./laragon` if `$HOME` is unset.
    pub fn default_root() -> PathBuf {
        match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join("laragon"),
            None => PathBuf::from("laragon"),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn www(&self) -> PathBuf {
        self.root.join("www")
    }
    pub fn etc(&self) -> PathBuf {
        self.root.join("etc")
    }
    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }
    pub fn log(&self) -> PathBuf {
        self.root.join("log")
    }
    pub fn tmp(&self) -> PathBuf {
        self.root.join("tmp")
    }
    pub fn ssl(&self) -> PathBuf {
        self.root.join("ssl")
    }
    pub fn etc_for(&self, sub: &str) -> PathBuf {
        self.etc().join(sub)
    }
    pub fn config_file(&self) -> PathBuf {
        self.root.join("laragon.toml")
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for dir in [self.www(), self.etc(), self.data(), self.log(), self.tmp(), self.ssl()] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p laragon-core paths`
Expected: PASS — 2 tests.

- [ ] **Step 5: Commit**

```bash
git add core/src/paths.rs
git commit -m "feat(core): add LaragonPaths directory layout"
```

---

### Task 3: Config — laragon.toml load/save

**Files:**
- Create: `core/src/config.rs`
- Modify: `core/src/lib.rs`

**Interfaces:**
- Consumes: `LaragonPaths::config_file`
- Produces:
  - `struct Config { tld: String, php_version: String, services: ServicesConfig }`
  - `struct ServicesConfig { nginx: bool, php: bool, mariadb: bool, redis: bool, mailpit: bool }`
  - `impl Default for Config` (tld `"dev"`, php `"8.4"`, all services `true`)
  - `Config::load(path: &Path) -> Result<Config, ConfigError>` (returns `Default` if file missing)
  - `Config::save(&self, path: &Path) -> Result<(), ConfigError>`
  - `enum ConfigError { Io(std::io::Error), Parse(toml::de::Error), Serialize(toml::ser::Error) }` via `thiserror`

- [ ] **Step 1: Add module declaration**

In `core/src/lib.rs` add after `pub mod paths;`:

```rust
pub mod config;
```

- [ ] **Step 2: Write the failing test**

Create `core/src/config.rs` with ONLY the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_dev_tld_and_php84() {
        let c = Config::default();
        assert_eq!(c.tld, "dev");
        assert_eq!(c.php_version, "8.4");
        assert!(c.services.nginx && c.services.php && c.services.mariadb);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let c = Config::load(std::path::Path::new("/no/such/laragon.toml")).unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let tmp = std::env::temp_dir().join(format!("lara-cfg-{}.toml", std::process::id()));
        let mut c = Config::default();
        c.tld = "test".into();
        c.php_version = "8.3".into();
        c.save(&tmp).unwrap();
        let back = Config::load(&tmp).unwrap();
        assert_eq!(c, back);
        std::fs::remove_file(&tmp).ok();
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laragon-core config`
Expected: FAIL — `cannot find type Config`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/config.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config parse error: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("config serialize error: {0}")]
    Serialize(#[from] toml::ser::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServicesConfig {
    pub nginx: bool,
    pub php: bool,
    pub mariadb: bool,
    pub redis: bool,
    pub mailpit: bool,
}

impl Default for ServicesConfig {
    fn default() -> Self {
        Self { nginx: true, php: true, mariadb: true, redis: true, mailpit: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_tld")]
    pub tld: String,
    #[serde(default = "default_php")]
    pub php_version: String,
    #[serde(default)]
    pub services: ServicesConfig,
}

fn default_tld() -> String {
    "dev".to_string()
}
fn default_php() -> String {
    "8.4".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self { tld: default_tld(), php_version: default_php(), services: ServicesConfig::default() }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(ConfigError::Io(e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laragon-core config`
Expected: PASS — 3 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/config.rs core/src/lib.rs
git commit -m "feat(core): add Config load/save (laragon.toml)"
```

---

### Task 4: Service trait + core service types

**Files:**
- Create: `core/src/service/mod.rs`
- Modify: `core/src/lib.rs`

**Interfaces:**
- Consumes: `LaragonPaths`
- Produces:
  - `enum ServiceKind { Nginx, PhpFpm, Mariadb, Redis, Mailpit }` (derive `Clone, Copy, PartialEq, Eq, Hash, Debug`)
  - `enum ServiceState { Stopped, Starting, Running, Stopping, Crashed }` (derive `Clone, Copy, PartialEq, Eq, Debug`)
  - `struct SpawnSpec { program: String, args: Vec<String>, env: Vec<(String, String)>, cwd: Option<PathBuf> }`
  - `enum ServiceError { Io(std::io::Error), Config(String), HealthCheck(String), Init(String) }` via `thiserror`
  - `trait Service: Send + Sync` with:
    - `fn kind(&self) -> ServiceKind;`
    - `fn name(&self) -> &str;`
    - `fn deps(&self) -> &[ServiceKind] { &[] }`
    - `fn write_config(&self, paths: &LaragonPaths) -> Result<(), ServiceError> { Ok(()) }`
    - `fn command(&self, paths: &LaragonPaths) -> SpawnSpec;`
    - `fn health_check(&self, paths: &LaragonPaths) -> Result<(), ServiceError>;`
    - `fn needs_init(&self, _paths: &LaragonPaths) -> bool { false }`
    - `fn init(&self, _paths: &LaragonPaths) -> Result<(), ServiceError> { Ok(()) }`

- [ ] **Step 1: Add module declaration**

In `core/src/lib.rs` add:

```rust
pub mod service;
```

- [ ] **Step 2: Write the failing test**

Create `core/src/service/mod.rs`. Put the test at the bottom; it defines a tiny in-file fake service to prove the trait is usable:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaragonPaths;

    struct Fake;
    impl Service for Fake {
        fn kind(&self) -> ServiceKind {
            ServiceKind::Redis
        }
        fn name(&self) -> &str {
            "fake"
        }
        fn command(&self, _paths: &LaragonPaths) -> SpawnSpec {
            SpawnSpec::new("true")
        }
        fn health_check(&self, _paths: &LaragonPaths) -> Result<(), ServiceError> {
            Ok(())
        }
    }

    #[test]
    fn trait_defaults_work() {
        let f = Fake;
        let p = LaragonPaths::new("/tmp/x".into());
        assert_eq!(f.name(), "fake");
        assert_eq!(f.kind(), ServiceKind::Redis);
        assert!(f.deps().is_empty());
        assert!(!f.needs_init(&p));
        assert_eq!(f.command(&p).program, "true");
    }

    #[test]
    fn spawnspec_builder_sets_fields() {
        let s = SpawnSpec::new("nginx").arg("-t").env("FOO", "bar");
        assert_eq!(s.program, "nginx");
        assert_eq!(s.args, vec!["-t".to_string()]);
        assert_eq!(s.env, vec![("FOO".to_string(), "bar".to_string())]);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laragon-core service`
Expected: FAIL — `cannot find type Service`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/service/mod.rs`:

```rust
use crate::paths::LaragonPaths;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ServiceKind {
    Nginx,
    PhpFpm,
    Mariadb,
    Redis,
    Mailpit,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ServiceState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config error: {0}")]
    Config(String),
    #[error("health check failed: {0}")]
    HealthCheck(String),
    #[error("init failed: {0}")]
    Init(String),
}

/// A fully-specified command to spawn a service process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
}

impl SpawnSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self { program: program.into(), args: Vec::new(), env: Vec::new(), cwd: None }
    }
    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }
    pub fn args<I, S>(mut self, items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(items.into_iter().map(Into::into));
        self
    }
    pub fn env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }
    pub fn cwd(mut self, dir: PathBuf) -> Self {
        self.cwd = Some(dir);
        self
    }
}

/// A managed service (nginx, php-fpm, mariadb, redis, mailpit).
pub trait Service: Send + Sync {
    fn kind(&self) -> ServiceKind;
    fn name(&self) -> &str;
    fn deps(&self) -> &[ServiceKind] {
        &[]
    }
    fn write_config(&self, _paths: &LaragonPaths) -> Result<(), ServiceError> {
        Ok(())
    }
    fn command(&self, paths: &LaragonPaths) -> SpawnSpec;
    fn health_check(&self, paths: &LaragonPaths) -> Result<(), ServiceError>;
    fn needs_init(&self, _paths: &LaragonPaths) -> bool {
        false
    }
    fn init(&self, _paths: &LaragonPaths) -> Result<(), ServiceError> {
        Ok(())
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laragon-core service`
Expected: PASS — 2 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/service/mod.rs core/src/lib.rs
git commit -m "feat(core): add Service trait and core service types"
```

---

### Task 5: ProcessSpawner abstraction + RealSpawner + FakeSpawner

**Files:**
- Create: `core/src/process.rs`
- Modify: `core/src/lib.rs`

**Interfaces:**
- Consumes: `SpawnSpec` (from `service`)
- Produces:
  - `trait ProcessSpawner: Send + Sync { fn spawn(&self, spec: &SpawnSpec) -> std::io::Result<Box<dyn Process>>; }`
  - `trait Process: Send + Sync { fn is_alive(&mut self) -> bool; fn stop(&mut self) -> std::io::Result<()>; fn pid(&self) -> u32; }`
  - `struct RealSpawner;` implementing `ProcessSpawner` via `std::process::Command` (sends SIGTERM on `stop`)
  - `struct FakeSpawner` + `struct FakeProcess` (test helper, `#[cfg(test)]`-free so other modules' tests can use it) recording spawned specs in an `Arc<Mutex<Vec<SpawnSpec>>>` and staying "alive" until `stop`

- [ ] **Step 1: Add module declaration**

In `core/src/lib.rs` add:

```rust
pub mod process;
```

- [ ] **Step 2: Write the failing test**

Create `core/src/process.rs` with the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::SpawnSpec;

    #[test]
    fn fake_spawner_records_and_tracks_alive() {
        let spawner = FakeSpawner::new();
        let log = spawner.log();
        let mut p = spawner.spawn(&SpawnSpec::new("redis-server").arg("--port").arg("6379")).unwrap();
        assert!(p.is_alive());
        assert_eq!(log.lock().unwrap().len(), 1);
        assert_eq!(log.lock().unwrap()[0].program, "redis-server");
        p.stop().unwrap();
        assert!(!p.is_alive());
    }

    #[test]
    fn real_spawner_runs_and_stops_a_process() {
        // `sleep 30` is a real long-lived process we can stop deterministically.
        let spawner = RealSpawner;
        let mut p = spawner.spawn(&SpawnSpec::new("sleep").arg("30")).unwrap();
        assert!(p.is_alive());
        assert!(p.pid() > 0);
        p.stop().unwrap();
        // Give the OS a moment, then confirm it is gone.
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!p.is_alive());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laragon-core process`
Expected: FAIL — `cannot find type FakeSpawner`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/process.rs`:

```rust
use crate::service::SpawnSpec;
use std::sync::{Arc, Mutex};

/// A running process handle.
pub trait Process: Send + Sync {
    fn is_alive(&mut self) -> bool;
    fn stop(&mut self) -> std::io::Result<()>;
    fn pid(&self) -> u32;
}

/// Spawns processes from a `SpawnSpec`. Hidden behind a trait so the
/// orchestrator can be tested without launching real binaries.
pub trait ProcessSpawner: Send + Sync {
    fn spawn(&self, spec: &SpawnSpec) -> std::io::Result<Box<dyn Process>>;
}

// ---------- Real implementation ----------

pub struct RealSpawner;

struct RealProcess {
    child: std::process::Child,
}

impl Process for RealProcess {
    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
    fn stop(&mut self) -> std::io::Result<()> {
        // Graceful SIGTERM via libc kill; fall back to SIGKILL if needed.
        let pid = self.child.id() as i32;
        unsafe {
            libc_kill(pid, 15); // SIGTERM
        }
        Ok(())
    }
    fn pid(&self) -> u32 {
        self.child.id()
    }
}

// Minimal libc kill binding to avoid a libc dependency for one call.
extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

impl ProcessSpawner for RealSpawner {
    fn spawn(&self, spec: &SpawnSpec) -> std::io::Result<Box<dyn Process>> {
        let mut cmd = std::process::Command::new(&spec.program);
        cmd.args(&spec.args);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        if let Some(dir) = &spec.cwd {
            cmd.current_dir(dir);
        }
        let child = cmd.spawn()?;
        Ok(Box::new(RealProcess { child }))
    }
}

// ---------- Fake implementation (used by other modules' tests) ----------

#[derive(Clone, Default)]
pub struct FakeSpawner {
    log: Arc<Mutex<Vec<SpawnSpec>>>,
}

impl FakeSpawner {
    pub fn new() -> Self {
        Self::default()
    }
    /// Shared record of every spec that was spawned, in order.
    pub fn log(&self) -> Arc<Mutex<Vec<SpawnSpec>>> {
        self.log.clone()
    }
}

pub struct FakeProcess {
    alive: bool,
    pid: u32,
}

impl Process for FakeProcess {
    fn is_alive(&mut self) -> bool {
        self.alive
    }
    fn stop(&mut self) -> std::io::Result<()> {
        self.alive = false;
        Ok(())
    }
    fn pid(&self) -> u32 {
        self.pid
    }
}

impl ProcessSpawner for FakeSpawner {
    fn spawn(&self, spec: &SpawnSpec) -> std::io::Result<Box<dyn Process>> {
        let mut log = self.log.lock().unwrap();
        log.push(spec.clone());
        let pid = 1000 + log.len() as u32;
        Ok(Box::new(FakeProcess { alive: true, pid }))
    }
}
```

Note: `FakeSpawner`/`FakeProcess` are intentionally NOT behind `#[cfg(test)]` so the orchestrator tests in Task 6 (a different module) can use them.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laragon-core process`
Expected: PASS — 2 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/process.rs core/src/lib.rs
git commit -m "feat(core): add ProcessSpawner abstraction with real and fake impls"
```

---

### Task 6: Orchestrator — start/stop a single service

**Files:**
- Create: `core/src/orchestrator.rs`
- Modify: `core/src/lib.rs`

**Interfaces:**
- Consumes: `Service`, `ServiceKind`, `ServiceState`, `ServiceError`, `ProcessSpawner`, `Process`, `LaragonPaths`
- Produces:
  - `struct Orchestrator`
  - `Orchestrator::new(paths: LaragonPaths, services: Vec<Box<dyn Service>>, spawner: Box<dyn ProcessSpawner>) -> Orchestrator`
  - `fn start(&mut self, kind: ServiceKind) -> Result<(), ServiceError>` — runs `needs_init`/`init`, `write_config`, spawns, records handle + sets `Running` (health-check is best-effort here; deeper probing happens via `refresh`)
  - `fn stop(&mut self, kind: ServiceKind) -> Result<(), ServiceError>` — stops handle, sets `Stopped`
  - `fn state(&self, kind: ServiceKind) -> ServiceState` — `Stopped` if unknown
  - `fn refresh(&mut self)` — for each running handle, if not alive mark `Crashed`

- [ ] **Step 1: Add module declaration**

In `core/src/lib.rs` add:

```rust
pub mod orchestrator;
```

- [ ] **Step 2: Write the failing test**

Create `core/src/orchestrator.rs` with the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaragonPaths;
    use crate::process::FakeSpawner;
    use crate::service::{Service, ServiceError, ServiceKind, SpawnSpec};

    struct Dummy {
        kind: ServiceKind,
        name: &'static str,
    }
    impl Service for Dummy {
        fn kind(&self) -> ServiceKind {
            self.kind
        }
        fn name(&self) -> &str {
            self.name
        }
        fn command(&self, _p: &LaragonPaths) -> SpawnSpec {
            SpawnSpec::new(self.name)
        }
        fn health_check(&self, _p: &LaragonPaths) -> Result<(), ServiceError> {
            Ok(())
        }
    }

    fn orch(spawner: FakeSpawner) -> Orchestrator {
        let services: Vec<Box<dyn Service>> =
            vec![Box::new(Dummy { kind: ServiceKind::Redis, name: "redis-server" })];
        Orchestrator::new(LaragonPaths::new("/tmp/lara".into()), services, Box::new(spawner))
    }

    #[test]
    fn unknown_service_is_stopped() {
        let o = orch(FakeSpawner::new());
        assert_eq!(o.state(ServiceKind::Nginx), ServiceState::Stopped);
    }

    #[test]
    fn start_then_stop_transitions_state() {
        let spawner = FakeSpawner::new();
        let log = spawner.log();
        let mut o = orch(spawner);

        o.start(ServiceKind::Redis).unwrap();
        assert_eq!(o.state(ServiceKind::Redis), ServiceState::Running);
        assert_eq!(log.lock().unwrap().len(), 1);

        o.stop(ServiceKind::Redis).unwrap();
        assert_eq!(o.state(ServiceKind::Redis), ServiceState::Stopped);
    }

    #[test]
    fn starting_unregistered_kind_errors() {
        let mut o = orch(FakeSpawner::new());
        assert!(o.start(ServiceKind::Nginx).is_err());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laragon-core orchestrator`
Expected: FAIL — `cannot find type Orchestrator`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/orchestrator.rs`:

```rust
use crate::paths::LaragonPaths;
use crate::process::{Process, ProcessSpawner};
use crate::service::{Service, ServiceError, ServiceKind, ServiceState};
use std::collections::HashMap;

pub struct Orchestrator {
    paths: LaragonPaths,
    services: Vec<Box<dyn Service>>,
    spawner: Box<dyn ProcessSpawner>,
    handles: HashMap<ServiceKind, Box<dyn Process>>,
    states: HashMap<ServiceKind, ServiceState>,
}

impl Orchestrator {
    pub fn new(
        paths: LaragonPaths,
        services: Vec<Box<dyn Service>>,
        spawner: Box<dyn ProcessSpawner>,
    ) -> Self {
        Self {
            paths,
            services,
            spawner,
            handles: HashMap::new(),
            states: HashMap::new(),
        }
    }

    fn find(&self, kind: ServiceKind) -> Option<&dyn Service> {
        self.services.iter().find(|s| s.kind() == kind).map(|b| b.as_ref())
    }

    pub fn state(&self, kind: ServiceKind) -> ServiceState {
        self.states.get(&kind).copied().unwrap_or(ServiceState::Stopped)
    }

    pub fn start(&mut self, kind: ServiceKind) -> Result<(), ServiceError> {
        let svc = self
            .find(kind)
            .ok_or_else(|| ServiceError::Config(format!("no such service: {kind:?}")))?;
        self.states.insert(kind, ServiceState::Starting);

        if svc.needs_init(&self.paths) {
            svc.init(&self.paths)?;
        }
        svc.write_config(&self.paths)?;
        let spec = svc.command(&self.paths);
        let handle = self.spawner.spawn(&spec)?;
        self.handles.insert(kind, handle);
        self.states.insert(kind, ServiceState::Running);
        Ok(())
    }

    pub fn stop(&mut self, kind: ServiceKind) -> Result<(), ServiceError> {
        if let Some(mut handle) = self.handles.remove(&kind) {
            self.states.insert(kind, ServiceState::Stopping);
            handle.stop()?;
        }
        self.states.insert(kind, ServiceState::Stopped);
        Ok(())
    }

    /// Mark any service whose process has died as `Crashed`.
    pub fn refresh(&mut self) {
        let dead: Vec<ServiceKind> = self
            .handles
            .iter_mut()
            .filter(|(_, h)| !h.is_alive())
            .map(|(k, _)| *k)
            .collect();
        for k in dead {
            self.handles.remove(&k);
            self.states.insert(k, ServiceState::Crashed);
        }
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laragon-core orchestrator`
Expected: PASS — 3 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/orchestrator.rs core/src/lib.rs
git commit -m "feat(core): add Orchestrator start/stop/state/refresh"
```

---

### Task 7: Orchestrator — start_all / stop_all with dependency ordering

**Files:**
- Modify: `core/src/orchestrator.rs`

**Interfaces:**
- Produces (added to `Orchestrator`):
  - `fn start_all(&mut self) -> Result<(), ServiceError>` — starts services in dependency order (a service starts only after all kinds in its `deps()` have started)
  - `fn stop_all(&mut self)` — stops all running services in reverse start order; never returns early on error
  - `fn start_order(&self) -> Vec<ServiceKind>` (pub for testing) — topological order of registered services by `deps()`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `core/src/orchestrator.rs` (extend the existing `mod tests`):

```rust
    struct DepDummy {
        kind: ServiceKind,
        deps: Vec<ServiceKind>,
    }
    impl Service for DepDummy {
        fn kind(&self) -> ServiceKind {
            self.kind
        }
        fn name(&self) -> &str {
            "dep"
        }
        fn deps(&self) -> &[ServiceKind] {
            &self.deps
        }
        fn command(&self, _p: &LaragonPaths) -> SpawnSpec {
            SpawnSpec::new(format!("{:?}", self.kind))
        }
        fn health_check(&self, _p: &LaragonPaths) -> Result<(), ServiceError> {
            Ok(())
        }
    }

    #[test]
    fn start_order_respects_deps() {
        let services: Vec<Box<dyn Service>> = vec![
            Box::new(DepDummy { kind: ServiceKind::Nginx, deps: vec![ServiceKind::PhpFpm] }),
            Box::new(DepDummy { kind: ServiceKind::PhpFpm, deps: vec![ServiceKind::Mariadb] }),
            Box::new(DepDummy { kind: ServiceKind::Mariadb, deps: vec![] }),
        ];
        let o = Orchestrator::new(
            LaragonPaths::new("/tmp/lara".into()),
            services,
            Box::new(FakeSpawner::new()),
        );
        let order = o.start_order();
        let pos = |k| order.iter().position(|x| *x == k).unwrap();
        assert!(pos(ServiceKind::Mariadb) < pos(ServiceKind::PhpFpm));
        assert!(pos(ServiceKind::PhpFpm) < pos(ServiceKind::Nginx));
    }

    #[test]
    fn start_all_spawns_every_service_in_order() {
        let spawner = FakeSpawner::new();
        let log = spawner.log();
        let services: Vec<Box<dyn Service>> = vec![
            Box::new(DepDummy { kind: ServiceKind::Nginx, deps: vec![ServiceKind::PhpFpm] }),
            Box::new(DepDummy { kind: ServiceKind::PhpFpm, deps: vec![] }),
        ];
        let mut o = Orchestrator::new(
            LaragonPaths::new("/tmp/lara".into()),
            services,
            Box::new(spawner),
        );
        o.start_all().unwrap();
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 2);
        // php-fpm (no deps) must be spawned before nginx.
        assert_eq!(log[0].program, "PhpFpm");
        assert_eq!(log[1].program, "Nginx");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core orchestrator`
Expected: FAIL — `no method named start_order`.

- [ ] **Step 3: Write minimal implementation**

Add these methods inside `impl Orchestrator` in `core/src/orchestrator.rs`:

```rust
    /// Topological order of registered services honoring `deps()`.
    /// Deterministic: respects registration order among independent services.
    pub fn start_order(&self) -> Vec<ServiceKind> {
        let mut ordered: Vec<ServiceKind> = Vec::new();
        let mut remaining: Vec<&dyn Service> = self.services.iter().map(|b| b.as_ref()).collect();

        while !remaining.is_empty() {
            // Find the first service whose deps are all already ordered.
            let idx = remaining.iter().position(|s| {
                s.deps().iter().all(|d| {
                    ordered.contains(d)
                        // Ignore deps on services we don't manage.
                        || !remaining.iter().any(|r| r.kind() == *d)
                })
            });
            match idx {
                Some(i) => {
                    let s = remaining.remove(i);
                    ordered.push(s.kind());
                }
                None => {
                    // Dependency cycle — break it deterministically.
                    let s = remaining.remove(0);
                    ordered.push(s.kind());
                }
            }
        }
        ordered
    }

    pub fn start_all(&mut self) -> Result<(), ServiceError> {
        for kind in self.start_order() {
            self.start(kind)?;
        }
        Ok(())
    }

    pub fn stop_all(&mut self) {
        for kind in self.start_order().into_iter().rev() {
            let _ = self.stop(kind);
        }
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p laragon-core orchestrator`
Expected: PASS — 5 tests total in the module.

- [ ] **Step 5: Commit**

```bash
git add core/src/orchestrator.rs
git commit -m "feat(core): add start_all/stop_all with dependency ordering"
```

---

### Task 8: Redis service

**Files:**
- Create: `core/src/service/redis.rs`
- Modify: `core/src/service/mod.rs` (add `pub mod redis;`)

**Interfaces:**
- Consumes: `Service`, `ServiceKind`, `SpawnSpec`, `ServiceError`, `LaragonPaths`
- Produces: `struct RedisService { port: u16 }` with `RedisService::new() -> Self` (default port 6379) implementing `Service`. `write_config` writes `etc/redis/redis.conf`; `command` runs `redis-server <conf>`; `health_check` probes `127.0.0.1:<port>`.

- [ ] **Step 1: Add a shared TCP-probe helper test + module decl**

In `core/src/service/mod.rs`, add at top-level (after the trait):

```rust
/// Returns Ok if a TCP connect to `127.0.0.1:port` succeeds within 1s.
pub fn probe_tcp(port: u16) -> Result<(), ServiceError> {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;
    let addr = ("127.0.0.1", port)
        .to_socket_addrs()
        .map_err(ServiceError::Io)?
        .next()
        .ok_or_else(|| ServiceError::HealthCheck("no address".into()))?;
    TcpStream::connect_timeout(&addr, Duration::from_secs(1))
        .map(|_| ())
        .map_err(|e| ServiceError::HealthCheck(format!("port {port}: {e}")))
}
```

Then add `pub mod redis;` to `core/src/service/mod.rs`.

- [ ] **Step 2: Write the failing test**

Create `core/src/service/redis.rs` with the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaragonPaths;
    use crate::service::{Service, ServiceKind};

    #[test]
    fn command_runs_redis_server_with_conf() {
        let p = LaragonPaths::new("/tmp/lara".into());
        let svc = RedisService::new();
        let spec = svc.command(&p);
        assert_eq!(spec.program, "redis-server");
        assert!(spec.args.iter().any(|a| a.ends_with("etc/redis/redis.conf")));
        assert_eq!(svc.kind(), ServiceKind::Redis);
    }

    #[test]
    fn write_config_creates_conf_with_port_and_dir() {
        let tmp = std::env::temp_dir().join(format!("lara-redis-{}", std::process::id()));
        let p = LaragonPaths::new(tmp.clone());
        let svc = RedisService::new();
        svc.write_config(&p).unwrap();
        let conf = std::fs::read_to_string(p.etc_for("redis").join("redis.conf")).unwrap();
        assert!(conf.contains("port 6379"));
        assert!(conf.contains("dir "));
        std::fs::remove_dir_all(&tmp).ok();
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laragon-core redis`
Expected: FAIL — `cannot find type RedisService`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/service/redis.rs`:

```rust
use crate::paths::LaragonPaths;
use crate::service::{probe_tcp, Service, ServiceError, ServiceKind, SpawnSpec};

pub struct RedisService {
    port: u16,
}

impl RedisService {
    pub fn new() -> Self {
        Self { port: 6379 }
    }
    fn conf_path(&self, paths: &LaragonPaths) -> std::path::PathBuf {
        paths.etc_for("redis").join("redis.conf")
    }
}

impl Default for RedisService {
    fn default() -> Self {
        Self::new()
    }
}

impl Service for RedisService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Redis
    }
    fn name(&self) -> &str {
        "redis"
    }
    fn write_config(&self, paths: &LaragonPaths) -> Result<(), ServiceError> {
        std::fs::create_dir_all(paths.etc_for("redis"))?;
        std::fs::create_dir_all(paths.data().join("redis"))?;
        let conf = format!(
            "port {port}\n\
             bind 127.0.0.1\n\
             dir {dir}\n\
             dbfilename dump.rdb\n\
             logfile {log}\n",
            port = self.port,
            dir = paths.data().join("redis").display(),
            log = paths.log().join("redis.log").display(),
        );
        std::fs::write(self.conf_path(paths), conf)?;
        Ok(())
    }
    fn command(&self, paths: &LaragonPaths) -> SpawnSpec {
        SpawnSpec::new("redis-server").arg(self.conf_path(paths).display().to_string())
    }
    fn health_check(&self, _paths: &LaragonPaths) -> Result<(), ServiceError> {
        probe_tcp(self.port)
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laragon-core redis`
Expected: PASS — 2 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/service/redis.rs core/src/service/mod.rs
git commit -m "feat(core): add RedisService"
```

---

### Task 9: Mailpit service

**Files:**
- Create: `core/src/service/mailpit.rs`
- Modify: `core/src/service/mod.rs` (add `pub mod mailpit;`)

**Interfaces:**
- Produces: `struct MailpitService { smtp_port: u16, ui_port: u16 }`, `MailpitService::new()` (smtp 1025, ui 8025) implementing `Service`. No config file needed; `command` runs `mailpit` with listen/SMTP flags; `health_check` probes the UI port.

- [ ] **Step 1: Add module decl**

Add `pub mod mailpit;` to `core/src/service/mod.rs`.

- [ ] **Step 2: Write the failing test**

Create `core/src/service/mailpit.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaragonPaths;
    use crate::service::{Service, ServiceKind};

    #[test]
    fn command_sets_listen_and_smtp_flags() {
        let p = LaragonPaths::new("/tmp/lara".into());
        let svc = MailpitService::new();
        let spec = svc.command(&p);
        assert_eq!(spec.program, "mailpit");
        let joined = spec.args.join(" ");
        assert!(joined.contains("--listen"));
        assert!(joined.contains("8025"));
        assert!(joined.contains("--smtp"));
        assert!(joined.contains("1025"));
        assert_eq!(svc.kind(), ServiceKind::Mailpit);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laragon-core mailpit`
Expected: FAIL — `cannot find type MailpitService`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/service/mailpit.rs`:

```rust
use crate::paths::LaragonPaths;
use crate::service::{probe_tcp, Service, ServiceError, ServiceKind, SpawnSpec};

pub struct MailpitService {
    smtp_port: u16,
    ui_port: u16,
}

impl MailpitService {
    pub fn new() -> Self {
        Self { smtp_port: 1025, ui_port: 8025 }
    }
}

impl Default for MailpitService {
    fn default() -> Self {
        Self::new()
    }
}

impl Service for MailpitService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Mailpit
    }
    fn name(&self) -> &str {
        "mailpit"
    }
    fn command(&self, _paths: &LaragonPaths) -> SpawnSpec {
        SpawnSpec::new("mailpit")
            .arg("--listen")
            .arg(format!("127.0.0.1:{}", self.ui_port))
            .arg("--smtp")
            .arg(format!("127.0.0.1:{}", self.smtp_port))
    }
    fn health_check(&self, _paths: &LaragonPaths) -> Result<(), ServiceError> {
        probe_tcp(self.ui_port)
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laragon-core mailpit`
Expected: PASS — 1 test.

- [ ] **Step 6: Commit**

```bash
git add core/src/service/mailpit.rs core/src/service/mod.rs
git commit -m "feat(core): add MailpitService"
```

---

### Task 10: PHP-FPM service

**Files:**
- Create: `core/src/service/php_fpm.rs`
- Modify: `core/src/service/mod.rs` (add `pub mod php_fpm;`)

**Interfaces:**
- Consumes: `Service` types, `LaragonPaths`
- Produces: `struct PhpFpmService { version: String }`, `PhpFpmService::new(version: impl Into<String>) -> Self`. `command` runs `php-fpm<version>` in foreground (`-F`) with `-y <pool conf>`; `write_config` writes `etc/php/<version>/php-fpm.conf` defining a pool listening on a unix socket in `tmp/`; `health_check` checks the socket file exists; `socket_path()` is `pub` so nginx (Task 12) can reference it. Deps: none.

- [ ] **Step 1: Add module decl**

Add `pub mod php_fpm;` to `core/src/service/mod.rs`.

- [ ] **Step 2: Write the failing test**

Create `core/src/service/php_fpm.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaragonPaths;
    use crate::service::{Service, ServiceKind};

    #[test]
    fn command_uses_versioned_binary_and_foreground() {
        let p = LaragonPaths::new("/tmp/lara".into());
        let svc = PhpFpmService::new("8.4");
        let spec = svc.command(&p);
        assert_eq!(spec.program, "php-fpm8.4");
        assert!(spec.args.contains(&"-F".to_string()));
        assert!(spec.args.iter().any(|a| a.ends_with("php-fpm.conf")));
        assert_eq!(svc.kind(), ServiceKind::PhpFpm);
    }

    #[test]
    fn socket_path_is_under_tmp() {
        let p = LaragonPaths::new("/tmp/lara".into());
        let svc = PhpFpmService::new("8.4");
        assert_eq!(svc.socket_path(&p), std::path::Path::new("/tmp/lara/tmp/php-fpm.sock"));
    }

    #[test]
    fn write_config_defines_pool_with_socket() {
        let tmp = std::env::temp_dir().join(format!("lara-php-{}", std::process::id()));
        let p = LaragonPaths::new(tmp.clone());
        let svc = PhpFpmService::new("8.4");
        svc.write_config(&p).unwrap();
        let conf =
            std::fs::read_to_string(p.etc_for("php").join("8.4").join("php-fpm.conf")).unwrap();
        assert!(conf.contains("[www]"));
        assert!(conf.contains("listen = "));
        assert!(conf.contains("php-fpm.sock"));
        std::fs::remove_dir_all(&tmp).ok();
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laragon-core php_fpm`
Expected: FAIL — `cannot find type PhpFpmService`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/service/php_fpm.rs`:

```rust
use crate::paths::LaragonPaths;
use crate::service::{Service, ServiceError, ServiceKind, SpawnSpec};
use std::path::PathBuf;

pub struct PhpFpmService {
    version: String,
}

impl PhpFpmService {
    pub fn new(version: impl Into<String>) -> Self {
        Self { version: version.into() }
    }
    pub fn socket_path(&self, paths: &LaragonPaths) -> PathBuf {
        paths.tmp().join("php-fpm.sock")
    }
    fn conf_path(&self, paths: &LaragonPaths) -> PathBuf {
        paths.etc_for("php").join(&self.version).join("php-fpm.conf")
    }
}

impl Service for PhpFpmService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::PhpFpm
    }
    fn name(&self) -> &str {
        "php-fpm"
    }
    fn write_config(&self, paths: &LaragonPaths) -> Result<(), ServiceError> {
        std::fs::create_dir_all(self.conf_path(paths).parent().unwrap())?;
        std::fs::create_dir_all(paths.tmp())?;
        let conf = format!(
            "[global]\n\
             pid = {pid}\n\
             error_log = {log}\n\
             daemonize = no\n\
             \n\
             [www]\n\
             user = {user}\n\
             listen = {sock}\n\
             listen.mode = 0660\n\
             pm = dynamic\n\
             pm.max_children = 10\n\
             pm.start_servers = 2\n\
             pm.min_spare_servers = 1\n\
             pm.max_spare_servers = 4\n",
            pid = paths.tmp().join("php-fpm.pid").display(),
            log = paths.log().join("php-fpm.log").display(),
            user = std::env::var("USER").unwrap_or_else(|_| "www-data".into()),
            sock = self.socket_path(paths).display(),
        );
        std::fs::write(self.conf_path(paths), conf)?;
        Ok(())
    }
    fn command(&self, paths: &LaragonPaths) -> SpawnSpec {
        SpawnSpec::new(format!("php-fpm{}", self.version))
            .arg("-F") // foreground, so the orchestrator owns the process
            .arg("-y")
            .arg(self.conf_path(paths).display().to_string())
    }
    fn health_check(&self, paths: &LaragonPaths) -> Result<(), ServiceError> {
        if self.socket_path(paths).exists() {
            Ok(())
        } else {
            Err(ServiceError::HealthCheck("php-fpm socket missing".into()))
        }
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laragon-core php_fpm`
Expected: PASS — 3 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/service/php_fpm.rs core/src/service/mod.rs
git commit -m "feat(core): add PhpFpmService"
```

---

### Task 11: MariaDB service (with first-run init)

**Files:**
- Create: `core/src/service/mariadb.rs`
- Modify: `core/src/service/mod.rs` (add `pub mod mariadb;`)

**Interfaces:**
- Produces: `struct MariadbService { port: u16 }`, `MariadbService::new()` (port 3306). `write_config` writes `etc/mariadb/my.cnf` (datadir = `data/mariadb`, socket in `tmp/`); `needs_init` returns true when `data/mariadb/mysql` dir is absent; `init` runs `mariadb-install-db --defaults-file=<my.cnf>`; `command` runs `mariadbd --defaults-file=<my.cnf>`; `health_check` probes the port.

- [ ] **Step 1: Add module decl**

Add `pub mod mariadb;` to `core/src/service/mod.rs`.

- [ ] **Step 2: Write the failing test**

Create `core/src/service/mariadb.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaragonPaths;
    use crate::service::{Service, ServiceKind};

    #[test]
    fn command_and_kind() {
        let p = LaragonPaths::new("/tmp/lara".into());
        let svc = MariadbService::new();
        let spec = svc.command(&p);
        assert_eq!(spec.program, "mariadbd");
        assert!(spec.args.iter().any(|a| a.contains("--defaults-file=")));
        assert_eq!(svc.kind(), ServiceKind::Mariadb);
    }

    #[test]
    fn needs_init_true_when_datadir_empty() {
        let tmp = std::env::temp_dir().join(format!("lara-maria-{}", std::process::id()));
        let p = LaragonPaths::new(tmp.clone());
        let svc = MariadbService::new();
        assert!(svc.needs_init(&p));
        std::fs::create_dir_all(p.data().join("mariadb").join("mysql")).unwrap();
        assert!(!svc.needs_init(&p));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn write_config_sets_datadir_and_port() {
        let tmp = std::env::temp_dir().join(format!("lara-mariacfg-{}", std::process::id()));
        let p = LaragonPaths::new(tmp.clone());
        let svc = MariadbService::new();
        svc.write_config(&p).unwrap();
        let conf = std::fs::read_to_string(p.etc_for("mariadb").join("my.cnf")).unwrap();
        assert!(conf.contains("datadir"));
        assert!(conf.contains("port=3306") || conf.contains("port = 3306"));
        std::fs::remove_dir_all(&tmp).ok();
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laragon-core mariadb`
Expected: FAIL — `cannot find type MariadbService`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/service/mariadb.rs`:

```rust
use crate::paths::LaragonPaths;
use crate::service::{probe_tcp, Service, ServiceError, ServiceKind, SpawnSpec};
use std::path::PathBuf;

pub struct MariadbService {
    port: u16,
}

impl MariadbService {
    pub fn new() -> Self {
        Self { port: 3306 }
    }
    fn cnf_path(&self, paths: &LaragonPaths) -> PathBuf {
        paths.etc_for("mariadb").join("my.cnf")
    }
    fn datadir(&self, paths: &LaragonPaths) -> PathBuf {
        paths.data().join("mariadb")
    }
}

impl Default for MariadbService {
    fn default() -> Self {
        Self::new()
    }
}

impl Service for MariadbService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Mariadb
    }
    fn name(&self) -> &str {
        "mariadb"
    }
    fn write_config(&self, paths: &LaragonPaths) -> Result<(), ServiceError> {
        std::fs::create_dir_all(paths.etc_for("mariadb"))?;
        std::fs::create_dir_all(self.datadir(paths))?;
        let conf = format!(
            "[mysqld]\n\
             datadir={datadir}\n\
             socket={sock}\n\
             port={port}\n\
             bind-address=127.0.0.1\n\
             pid-file={pid}\n\
             log-error={log}\n",
            datadir = self.datadir(paths).display(),
            sock = paths.tmp().join("mysql.sock").display(),
            port = self.port,
            pid = paths.tmp().join("mariadb.pid").display(),
            log = paths.log().join("mariadb.log").display(),
        );
        std::fs::write(self.cnf_path(paths), conf)?;
        Ok(())
    }
    fn needs_init(&self, paths: &LaragonPaths) -> bool {
        !self.datadir(paths).join("mysql").is_dir()
    }
    fn init(&self, paths: &LaragonPaths) -> Result<(), ServiceError> {
        self.write_config(paths)?;
        let status = std::process::Command::new("mariadb-install-db")
            .arg(format!("--defaults-file={}", self.cnf_path(paths).display()))
            .arg(format!("--datadir={}", self.datadir(paths).display()))
            .arg("--auth-root-authentication-method=normal")
            .status()
            .map_err(|e| ServiceError::Init(format!("mariadb-install-db: {e}")))?;
        if !status.success() {
            return Err(ServiceError::Init("mariadb-install-db failed".into()));
        }
        Ok(())
    }
    fn command(&self, paths: &LaragonPaths) -> SpawnSpec {
        SpawnSpec::new("mariadbd")
            .arg(format!("--defaults-file={}", self.cnf_path(paths).display()))
    }
    fn health_check(&self, _paths: &LaragonPaths) -> Result<(), ServiceError> {
        probe_tcp(self.port)
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laragon-core mariadb`
Expected: PASS — 3 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/service/mariadb.rs core/src/service/mod.rs
git commit -m "feat(core): add MariadbService with first-run init"
```

---

### Task 12: Nginx service

**Files:**
- Create: `core/src/service/nginx.rs`
- Modify: `core/src/service/mod.rs` (add `pub mod nginx;`)

**Interfaces:**
- Consumes: `PhpFpmService::socket_path` (to wire fastcgi), `Service` types, `LaragonPaths`
- Produces: `struct NginxService { http_port: u16, php_socket: PathBuf }`, `NginxService::new(php_socket: PathBuf) -> Self` (http port 80). `deps()` returns `[PhpFpm]`. `write_config` writes `etc/nginx/nginx.conf` with a default server on port 80 serving `www/` and a `fastcgi_pass unix:<php_socket>` PHP location, plus an `include` for `etc/nginx/sites/*.conf`. `command` runs `nginx -p <etc/nginx> -c <nginx.conf> -g 'daemon off;'`. `health_check` probes the http port.

- [ ] **Step 1: Add module decl**

Add `pub mod nginx;` to `core/src/service/mod.rs`.

- [ ] **Step 2: Write the failing test**

Create `core/src/service/nginx.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaragonPaths;
    use crate::service::{Service, ServiceKind};

    #[test]
    fn command_runs_nginx_with_prefix_and_daemon_off() {
        let p = LaragonPaths::new("/tmp/lara".into());
        let svc = NginxService::new("/tmp/lara/tmp/php-fpm.sock".into());
        let spec = svc.command(&p);
        assert_eq!(spec.program, "nginx");
        let joined = spec.args.join(" ");
        assert!(joined.contains("-p"));
        assert!(joined.contains("daemon off;"));
        assert_eq!(svc.kind(), ServiceKind::Nginx);
        assert_eq!(svc.deps(), &[ServiceKind::PhpFpm]);
    }

    #[test]
    fn write_config_wires_php_socket_and_includes_sites() {
        let tmp = std::env::temp_dir().join(format!("lara-nginx-{}", std::process::id()));
        let p = LaragonPaths::new(tmp.clone());
        let sock = p.tmp().join("php-fpm.sock");
        let svc = NginxService::new(sock.clone());
        svc.write_config(&p).unwrap();
        let conf = std::fs::read_to_string(p.etc_for("nginx").join("nginx.conf")).unwrap();
        assert!(conf.contains(&format!("fastcgi_pass unix:{}", sock.display())));
        assert!(conf.contains("listen 80"));
        assert!(conf.contains("sites/*.conf"));
        // sites dir must exist so the glob include doesn't error.
        assert!(p.etc_for("nginx").join("sites").is_dir());
        std::fs::remove_dir_all(&tmp).ok();
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laragon-core nginx`
Expected: FAIL — `cannot find type NginxService`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/service/nginx.rs`:

```rust
use crate::paths::LaragonPaths;
use crate::service::{probe_tcp, Service, ServiceError, ServiceKind, SpawnSpec};
use std::path::PathBuf;

pub struct NginxService {
    http_port: u16,
    php_socket: PathBuf,
}

impl NginxService {
    pub fn new(php_socket: PathBuf) -> Self {
        Self { http_port: 80, php_socket }
    }
    fn conf_path(&self, paths: &LaragonPaths) -> PathBuf {
        paths.etc_for("nginx").join("nginx.conf")
    }
}

impl Service for NginxService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Nginx
    }
    fn name(&self) -> &str {
        "nginx"
    }
    fn deps(&self) -> &[ServiceKind] {
        const DEPS: [ServiceKind; 1] = [ServiceKind::PhpFpm];
        &DEPS
    }
    fn write_config(&self, paths: &LaragonPaths) -> Result<(), ServiceError> {
        std::fs::create_dir_all(paths.etc_for("nginx").join("sites"))?;
        std::fs::create_dir_all(paths.tmp())?;
        std::fs::create_dir_all(paths.log())?;
        let conf = format!(
            "worker_processes auto;\n\
             pid {pid};\n\
             error_log {errlog};\n\
             events {{ worker_connections 1024; }}\n\
             http {{\n\
             \x20 access_log {acclog};\n\
             \x20 client_body_temp_path {tmp}/nginx-client;\n\
             \x20 proxy_temp_path {tmp}/nginx-proxy;\n\
             \x20 fastcgi_temp_path {tmp}/nginx-fastcgi;\n\
             \x20 default_type application/octet-stream;\n\
             \x20 server {{\n\
             \x20   listen {port};\n\
             \x20   server_name localhost;\n\
             \x20   root {www};\n\
             \x20   index index.php index.html;\n\
             \x20   location / {{ try_files $uri $uri/ /index.php?$query_string; }}\n\
             \x20   location ~ \\.php$ {{\n\
             \x20     fastcgi_pass unix:{sock};\n\
             \x20     fastcgi_index index.php;\n\
             \x20     include {nginx_etc}/fastcgi_params;\n\
             \x20     fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\n\
             \x20   }}\n\
             \x20 }}\n\
             \x20 include {nginx_etc}/sites/*.conf;\n\
             }}\n",
            pid = paths.tmp().join("nginx.pid").display(),
            errlog = paths.log().join("nginx-error.log").display(),
            acclog = paths.log().join("nginx-access.log").display(),
            tmp = paths.tmp().display(),
            port = self.http_port,
            www = paths.www().display(),
            sock = self.php_socket.display(),
            nginx_etc = paths.etc_for("nginx").display(),
        );
        std::fs::write(self.conf_path(paths), conf)?;
        // Provide a minimal fastcgi_params so the include resolves.
        std::fs::write(
            paths.etc_for("nginx").join("fastcgi_params"),
            "fastcgi_param QUERY_STRING $query_string;\n\
             fastcgi_param REQUEST_METHOD $request_method;\n\
             fastcgi_param CONTENT_TYPE $content_type;\n\
             fastcgi_param CONTENT_LENGTH $content_length;\n\
             fastcgi_param REQUEST_URI $request_uri;\n\
             fastcgi_param DOCUMENT_URI $document_uri;\n\
             fastcgi_param DOCUMENT_ROOT $document_root;\n\
             fastcgi_param SERVER_PROTOCOL $server_protocol;\n\
             fastcgi_param GATEWAY_INTERFACE CGI/1.1;\n\
             fastcgi_param REMOTE_ADDR $remote_addr;\n\
             fastcgi_param SERVER_NAME $server_name;\n",
        )?;
        Ok(())
    }
    fn command(&self, paths: &LaragonPaths) -> SpawnSpec {
        SpawnSpec::new("nginx")
            .arg("-p")
            .arg(paths.etc_for("nginx").display().to_string())
            .arg("-c")
            .arg(self.conf_path(paths).display().to_string())
            .arg("-g")
            .arg("daemon off;")
    }
    fn health_check(&self, _paths: &LaragonPaths) -> Result<(), ServiceError> {
        probe_tcp(self.http_port)
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laragon-core nginx`
Expected: PASS — 2 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/service/nginx.rs core/src/service/mod.rs
git commit -m "feat(core): add NginxService"
```

---

### Task 13: Service registry from Config

**Files:**
- Create: `core/src/service/registry.rs`
- Modify: `core/src/service/mod.rs` (add `pub mod registry;`)

**Interfaces:**
- Consumes: `Config`, all five service structs, `PhpFpmService::socket_path`, `LaragonPaths`
- Produces: `fn build_services(config: &Config, paths: &LaragonPaths) -> Vec<Box<dyn Service>>` — includes only services enabled in `config.services`; wires nginx's fastcgi socket to the php-fpm socket for `config.php_version`.

- [ ] **Step 1: Add module decl**

Add `pub mod registry;` to `core/src/service/mod.rs`.

- [ ] **Step 2: Write the failing test**

Create `core/src/service/registry.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::paths::LaragonPaths;
    use crate::service::ServiceKind;

    #[test]
    fn builds_all_enabled_services() {
        let cfg = Config::default();
        let p = LaragonPaths::new("/tmp/lara".into());
        let svcs = build_services(&cfg, &p);
        let kinds: Vec<ServiceKind> = svcs.iter().map(|s| s.kind()).collect();
        for k in [
            ServiceKind::Nginx,
            ServiceKind::PhpFpm,
            ServiceKind::Mariadb,
            ServiceKind::Redis,
            ServiceKind::Mailpit,
        ] {
            assert!(kinds.contains(&k), "missing {k:?}");
        }
    }

    #[test]
    fn omits_disabled_services() {
        let mut cfg = Config::default();
        cfg.services.redis = false;
        cfg.services.mailpit = false;
        let p = LaragonPaths::new("/tmp/lara".into());
        let kinds: Vec<ServiceKind> =
            build_services(&cfg, &p).iter().map(|s| s.kind()).collect();
        assert!(!kinds.contains(&ServiceKind::Redis));
        assert!(!kinds.contains(&ServiceKind::Mailpit));
        assert!(kinds.contains(&ServiceKind::Nginx));
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laragon-core registry`
Expected: FAIL — `cannot find function build_services`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/service/registry.rs`:

```rust
use crate::config::Config;
use crate::paths::LaragonPaths;
use crate::service::mailpit::MailpitService;
use crate::service::mariadb::MariadbService;
use crate::service::nginx::NginxService;
use crate::service::php_fpm::PhpFpmService;
use crate::service::redis::RedisService;
use crate::service::Service;

/// Build the set of services enabled in `config`, wiring nginx to the
/// php-fpm socket for the configured PHP version.
pub fn build_services(config: &Config, paths: &LaragonPaths) -> Vec<Box<dyn Service>> {
    let mut services: Vec<Box<dyn Service>> = Vec::new();
    let php = PhpFpmService::new(config.php_version.clone());
    let php_socket = php.socket_path(paths);

    if config.services.mariadb {
        services.push(Box::new(MariadbService::new()));
    }
    if config.services.redis {
        services.push(Box::new(RedisService::new()));
    }
    if config.services.php {
        services.push(Box::new(php));
    }
    if config.services.nginx {
        services.push(Box::new(NginxService::new(php_socket)));
    }
    if config.services.mailpit {
        services.push(Box::new(MailpitService::new()));
    }
    services
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laragon-core registry`
Expected: PASS — 2 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/service/registry.rs core/src/service/mod.rs
git commit -m "feat(core): add service registry from Config"
```

---

### Task 14: `laragonctl` CLI wiring + manual smoke test

**Files:**
- Modify: `laragonctl/src/main.rs`
- Modify: `core/src/lib.rs` (add a small `prelude`/re-exports for convenience)

**Interfaces:**
- Consumes: `LaragonPaths`, `Config`, `build_services`, `Orchestrator`, `RealSpawner`, `ServiceKind`
- Produces: a CLI with subcommands `up`, `down`, `status`, `config-init`. No external arg-parser crate — hand-rolled `match` on `std::env::args`.

- [ ] **Step 1: Add convenience re-exports to `core/src/lib.rs`**

Append:

```rust
pub use config::Config;
pub use orchestrator::Orchestrator;
pub use paths::LaragonPaths;
pub use process::RealSpawner;
pub use service::registry::build_services;
pub use service::ServiceKind;
```

- [ ] **Step 2: Write the CLI**

Replace `laragonctl/src/main.rs` with:

```rust
use laragon_core::{build_services, Config, LaragonPaths, Orchestrator, RealSpawner, ServiceKind};

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "help".into());
    let paths = LaragonPaths::new(LaragonPaths::default_root());

    match cmd.as_str() {
        "config-init" => {
            paths.ensure_dirs().expect("create dirs");
            let cfg = Config::default();
            cfg.save(&paths.config_file()).expect("save config");
            println!("Initialized {}", paths.config_file().display());
        }
        "up" => {
            let cfg = Config::load(&paths.config_file()).expect("load config");
            paths.ensure_dirs().expect("create dirs");
            let mut orch =
                Orchestrator::new(paths.clone(), build_services(&cfg, &paths), Box::new(RealSpawner));
            match orch.start_all() {
                Ok(()) => println!("Started all services. Press Ctrl-C to stop."),
                Err(e) => {
                    eprintln!("start failed: {e}");
                    orch.stop_all();
                    std::process::exit(1);
                }
            }
            // Keep the process (and thus child processes) alive until Ctrl-C.
            wait_for_ctrl_c();
            println!("Stopping...");
            orch.stop_all();
        }
        "status" => {
            let cfg = Config::load(&paths.config_file()).expect("load config");
            let orch =
                Orchestrator::new(paths.clone(), build_services(&cfg, &paths), Box::new(RealSpawner));
            for kind in orch.start_order() {
                println!("{:?}: {:?}", kind, orch.state(kind));
            }
        }
        "down" => {
            println!("`up` manages the process lifetime; stop it with Ctrl-C.");
        }
        _ => {
            println!("usage: laragonctl <config-init|up|status>");
        }
    }
}

fn wait_for_ctrl_c() {
    use std::sync::mpsc::channel;
    let (tx, rx) = channel();
    ctrlc_lite(move || {
        let _ = tx.send(());
    });
    let _ = rx.recv();
}

// Minimal Ctrl-C handler without external crates: register a SIGINT handler
// that writes to a static flag via a self-pipe-free approach using libc.
fn ctrlc_lite<F: Fn() + Send + 'static>(f: F) {
    use std::sync::Mutex;
    use std::sync::OnceLock;
    static HANDLER: OnceLock<Mutex<Option<Box<dyn Fn() + Send>>>> = OnceLock::new();
    HANDLER.get_or_init(|| Mutex::new(None));
    *HANDLER.get().unwrap().lock().unwrap() = Some(Box::new(f));

    extern "C" fn on_sigint(_sig: i32) {
        if let Some(lock) = HANDLER.get() {
            if let Ok(guard) = lock.lock() {
                if let Some(cb) = guard.as_ref() {
                    cb();
                }
            }
        }
    }
    extern "C" {
        fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    }
    unsafe {
        signal(2, on_sigint); // SIGINT = 2
    }
}
```

- [ ] **Step 3: Build the CLI**

Run: `cargo build -p laragonctl`
Expected: PASS — compiles.

- [ ] **Step 4: Manual smoke test (requires the stack binaries installed)**

This step validates real-world behavior. It needs `nginx`, `php-fpm8.4`, `mariadbd`, `redis-server`, `mailpit` on `PATH`. If they are not installed yet, document that and defer the live run to after Plan 3's setup wizard; the `cargo test` suite already covers all logic.

Manual checklist (run from repo root):

```bash
cargo run -p laragonctl -- config-init
# Put a quick PHP file to serve:
echo '<?php phpinfo();' > ~/laragon/www/index.php
cargo run -p laragonctl -- up
# In another terminal:
curl -s http://localhost/ | head -1     # expect HTML from phpinfo
redis-cli -p 6379 ping                   # expect PONG
curl -s http://localhost:8025/ | head -1 # expect Mailpit UI HTML
# Ctrl-C the `up` terminal; confirm no orphan processes:
pgrep -a 'nginx|php-fpm|mariadbd|redis-server|mailpit'   # expect empty
```

Expected: `curl http://localhost/` returns phpinfo HTML; `redis-cli ping` returns `PONG`; after Ctrl-C no stack processes remain.

- [ ] **Step 5: Commit**

```bash
git add laragonctl/src/main.rs core/src/lib.rs
git commit -m "feat(laragonctl): wire CLI up/down/status/config-init to core"
```

---

## Self-Review

**1. Spec coverage (Plan 1 scope = headless core orchestration):**
- Directory layout (spec §3) → Task 2 ✓
- `laragon.toml` config (spec §2, Global Constraints) → Task 3 ✓
- Service trait / per-service config generation, no `/etc` (spec §3, §4) → Tasks 4, 8–12 ✓
- App-managed processes, own data/config (Phương án A, spec §2) → Tasks 5, 6 ✓
- Lifecycle states + dependency start order mariadb/redis→php→nginx (spec §4, Global Constraints) → Tasks 6, 7, 12 (`deps()`), 13 ✓
- Crash detection (spec §8) → Task 6 `refresh()` ✓
- MariaDB first-run init (spec §4) → Task 11 ✓
- Health checks (spec §4) → `probe_tcp` + per-service `health_check` (Tasks 8–12) ✓
- Working software exercising the stack → Task 14 `laragonctl` ✓
- TDD throughout (spec §9) → every task ✓
- **Deferred to later plans (correctly out of Plan 1 scope):** sites/`www` scan, vhost-per-site, `/etc/hosts`, mkcert SSL, `setcap`, privileged/polkit, apt install, GUI/tray, setup wizard → Plans 2 & 3.

**2. Placeholder scan:** No "TBD/TODO/handle edge cases" left; every code step shows complete code. Task 14 Step 4 is an explicit manual checklist with concrete commands and is allowed to note deferral if binaries are absent. ✓

**3. Type consistency:** `LaragonPaths`, `Config`/`ServicesConfig`, `Service` trait method set, `SpawnSpec` builder (`new/arg/args/env/cwd`), `ProcessSpawner::spawn -> Box<dyn Process>`, `Process::{is_alive,stop,pid}`, `Orchestrator::{new,start,stop,state,refresh,start_order,start_all,stop_all}`, `build_services(&Config,&LaragonPaths)`, `PhpFpmService::socket_path`, `ServiceKind` variants — all names/signatures used consistently across tasks. ✓

**Note on `RealProcess::stop`:** uses an `extern "C" kill` binding to avoid adding the `libc` crate. If the implementer prefers, adding `libc = "0.2"` to `core/Cargo.toml` and calling `libc::kill` is an acceptable equivalent — keep whichever compiles cleanly on the target.
