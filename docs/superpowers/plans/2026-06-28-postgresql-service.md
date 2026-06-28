# PostgreSQL Opt-in Managed Service — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add PostgreSQL as a no-apt, multi-version, opt-in managed service (off by default), with a service enable/disable toggle UI, orchestrated like MariaDB and reachable from DbGate.

**Architecture:** A `postgres_static.rs` installer unwraps the zonky embedded-postgres jar into `bin/postgres/<ver>/`; a `PostgresService` (stateful, initdb on first start) plugs into the orchestrator; a new `orchestrator.reconcile` + `set_service_enabled` command + a Settings "Services" toggle section make it opt-in; the dashboard renders its service grid from the enabled set so PostgreSQL appears/disappears.

**Tech Stack:** Rust (laralux-core, laralux-desktop/Tauri 2), `zip` crate, TypeScript + Vite frontend.

## Global Constraints

- `laralux-core` keeps ZERO Tauri dependencies (the `zip` crate is fine — not Tauri).
- No-apt: install only static/portable binaries downloaded into `~/laralux/`; no `apt`, no compiler, no `unzip`/system-zip dependency (use the `zip` crate in Rust).
- PostgreSQL is **opt-in**: `ServicesConfig.postgres` defaults to `false`.
- Superuser `postgres`, **no password**, local `trust` auth (the PG analogue of MariaDB's password-less root). Port `5432`, bind `127.0.0.1`.
- Zonky source: `https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-linux-<arch>/<ver>/embedded-postgres-binaries-linux-<arch>-<ver>.jar`, `<arch>` ∈ {`amd64`,`arm64v8`}; the jar holds one `*.txz` of a standard PG layout (`bin/postgres`, `bin/initdb`, `bin/psql`, …).
- Versions: `KNOWN_POSTGRES_VERSIONS = ["17.2.0","16.6.0","15.8.0","14.13.0"]`, default `"16.6.0"`.
- Git commits: NO `Co-Authored-By` trailer.
- Build commands need cargo on PATH: `export PATH="$HOME/.cargo/bin:$PATH"`. Frontend build = `npm run build` (`tsc --noEmit && vite build`). DO NOT create a git worktree — work on master.

---

### Task 1: Core — `postgres_static.rs` installer (zonky jar → bin/postgres/<ver>)

**Files:**
- Create: `core/src/postgres_static.rs`
- Modify: `core/Cargo.toml` (add `zip` dep)
- Modify: `core/src/lib.rs` (declare module + re-export)

**Interfaces:**
- Consumes: `LaraluxPaths::{version_dir, tmp}`, `Downloader::fetch_with_progress`, `CommandRunner::run`, `ProgressSink`, `crate::layout::set_current`.
- Produces: `pub const POSTGRES_VERSION`, `pub const KNOWN_POSTGRES_VERSIONS`, `pub fn postgres_arch() -> Option<&'static str>`, `pub fn postgres_url(version, arch) -> String`, `pub fn install_postgres(...) -> Result<String, PostgresError>`, `pub fn install_postgres_version(paths, version, downloader, runner, sink) -> Result<String, PostgresError>`, `pub enum PostgresError`.

- [ ] **Step 1: Add the `zip` dependency** — in `core/Cargo.toml`, under `[dependencies]`, add:

```toml
zip = "2"
```

- [ ] **Step 2: Write the failing tests** — create `core/src/postgres_static.rs` with ONLY the tests first (so it fails to compile/run):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_and_arch() {
        assert_eq!(
            postgres_url("16.6.0", "amd64"),
            "https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-linux-amd64/16.6.0/embedded-postgres-binaries-linux-amd64-16.6.0.jar"
        );
        assert_eq!(
            postgres_url("16.6.0", "arm64v8"),
            "https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-linux-arm64v8/16.6.0/embedded-postgres-binaries-linux-arm64v8-16.6.0.jar"
        );
        assert_eq!(
            postgres_arch(),
            match std::env::consts::ARCH { "x86_64" => Some("amd64"), "aarch64" => Some("arm64v8"), _ => None }
        );
    }

    #[test]
    fn known_versions_include_pinned_default() {
        assert!(KNOWN_POSTGRES_VERSIONS.contains(&POSTGRES_VERSION));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p laralux-core postgres_static 2>&1 | tail -15`
Expected: FAIL — module not declared / functions not found.

- [ ] **Step 4: Implement the installer** — prepend (above the `#[cfg(test)]`) in `core/src/postgres_static.rs`:

```rust
use crate::paths::LaraluxPaths;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;
use std::io::Read;

pub const POSTGRES_VERSION: &str = "16.6.0";

/// Curated PostgreSQL versions offered in the Setup/version modal. All verified
/// present on Maven Central as zonky `embedded-postgres-binaries-linux-<arch>`
/// jars. The datadir is version-sensitive — a major downgrade may refuse to start.
pub const KNOWN_POSTGRES_VERSIONS: [&str; 4] = ["17.2.0", "16.6.0", "15.8.0", "14.13.0"];

#[derive(Debug, thiserror::Error)]
pub enum PostgresError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn postgres_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("amd64"), "aarch64" => Some("arm64v8"), _ => None }
}

pub fn postgres_url(version: &str, arch: &str) -> String {
    format!(
        "https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-linux-{arch}/{version}/embedded-postgres-binaries-linux-{arch}-{version}.jar"
    )
}

/// Make `link` (under basedir) a relative symlink to `target_rel`.
fn rel_symlink(basedir: &std::path::Path, link_name: &str, target_rel: &str) -> std::io::Result<()> {
    let link = basedir.join(link_name);
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    { std::os::unix::fs::symlink(target_rel, &link)?; }
    Ok(())
}

/// Download + install the default (pinned) PostgreSQL version.
pub fn install_postgres(
    paths: &LaraluxPaths, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, PostgresError> {
    install_postgres_version(paths, POSTGRES_VERSION, downloader, runner, sink)
}

/// Download the zonky jar, extract the inner `*.txz` (pure-Rust zip), untar it
/// (xz) into bin/postgres/<version>/ (yielding bin/, lib/, share/), and create
/// top-level symlinks to the executables. Idempotent.
pub fn install_postgres_version(
    paths: &LaraluxPaths, version: &str, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, PostgresError> {
    let basedir = paths.version_dir("postgres", version);
    if basedir.join("bin").join("postgres").exists() {
        let _ = crate::layout::set_current(paths, "postgres", version);
        return Ok(version.to_string());
    }
    let arch = postgres_arch().ok_or_else(|| PostgresError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    let jar = paths.tmp().join("postgres.jar");
    downloader.fetch_with_progress(&postgres_url(version, arch), &jar, sink)
        .map_err(|e| PostgresError::Download(e.to_string()))?;

    // Extract the single `*.txz` entry from the jar (a zip) — no `unzip` dependency.
    let txz = paths.tmp().join("postgres.txz");
    {
        let f = std::fs::File::open(&jar)?;
        let mut zipf = zip::ZipArchive::new(f).map_err(|e| PostgresError::Extract(e.to_string()))?;
        let name = (0..zipf.len())
            .filter_map(|i| zipf.by_index(i).ok().map(|e| e.name().to_string()))
            .find(|n| n.ends_with(".txz"))
            .ok_or_else(|| PostgresError::Extract("no .txz entry in jar".into()))?;
        let mut entry = zipf.by_name(&name).map_err(|e| PostgresError::Extract(e.to_string()))?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        std::fs::write(&txz, &buf)?;
    }

    // Untar the xz tarball into the version dir (PG layout: bin/, lib/, share/).
    let _ = std::fs::remove_dir_all(&basedir);
    std::fs::create_dir_all(&basedir)?;
    runner.run("tar", &["-xJf".into(), txz.display().to_string(), "-C".into(), basedir.display().to_string()], None)
        .map_err(|e| PostgresError::Extract(e.to_string()))?;
    if !basedir.join("bin").join("postgres").exists() {
        return Err(PostgresError::Extract("postgres binary missing after extract".into()));
    }

    // Top-level symlinks so bin/postgres/current/<exe> resolves (resolver + CLI symlinks).
    for exe in ["postgres", "initdb", "pg_ctl", "psql", "pg_dump", "pg_restore", "createdb", "dropdb"] {
        let _ = rel_symlink(&basedir, exe, &format!("bin/{exe}"));
    }
    let _ = std::fs::remove_file(&jar);
    let _ = std::fs::remove_file(&txz);
    crate::layout::set_current(paths, "postgres", version)
        .map_err(|e| PostgresError::Extract(e.to_string()))?;
    Ok(version.to_string())
}
```

- [ ] **Step 5: Declare module + re-export** — in `core/src/lib.rs`, add after `pub mod node_static;` (line ~30):

```rust
pub mod postgres_static;
```

and after `pub use mariadb_static::{install_mariadb, MariadbError};` (line ~71):

```rust
pub use postgres_static::{install_postgres, PostgresError};
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p laralux-core postgres_static 2>&1 | tail -15`
Expected: PASS — `url_and_arch`, `known_versions_include_pinned_default`.

- [ ] **Step 7: Commit**

```bash
git add core/src/postgres_static.rs core/src/lib.rs core/Cargo.toml core/Cargo.lock
git commit -m "feat(core): postgres_static installer (zonky portable binaries)"
```

---

### Task 2: Core — `PostgresService`

**Files:**
- Create: `core/src/service/postgres.rs`
- Modify: `core/src/service/mod.rs` (declare module; add `ServiceKind::Postgres`)

**Interfaces:**
- Consumes: `Service` trait, `ServiceKind`, `SpawnSpec`, `probe_tcp`, `cleanup_stale_endpoint`, `crate::bin::resolve_bin`, `crate::layout::managed_bin_dirs`, `LaraluxPaths::{data,tmp,log}`.
- Produces: `pub struct PostgresService` implementing `Service` with `kind()==ServiceKind::Postgres`, `name()=="postgres"`.

- [ ] **Step 1: Add the enum variant** — in `core/src/service/mod.rs`, add `Postgres,` to `ServiceKind` (after `Mariadb,`):

```rust
pub enum ServiceKind {
    Nginx,
    PhpFpm,
    Mariadb,
    Postgres,
    Redis,
    Mailpit,
    Coredns,
}
```

- [ ] **Step 2: Write the failing tests** — create `core/src/service/postgres.rs`:

```rust
use crate::paths::LaraluxPaths;
use crate::service::{probe_tcp, Service, ServiceError, ServiceKind, SpawnSpec};
use std::path::PathBuf;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Service, ServiceKind};

    #[test]
    fn command_and_kind() {
        let p = LaraluxPaths::new("/tmp/lara".into());
        let svc = PostgresService::new();
        let spec = svc.command(&p);
        assert_eq!(spec.program, "postgres");
        assert!(spec.args.iter().any(|a| a == "-D"));
        assert!(spec.args.iter().any(|a| a == "5432"));
        assert!(spec.args.iter().any(|a| a == "listen_addresses=127.0.0.1"));
        assert!(spec.args.iter().any(|a| a == "logging_collector=on"));
        assert!(spec.args.iter().any(|a| a == "log_filename=postgres.log"));
        assert_eq!(svc.kind(), ServiceKind::Postgres);
    }

    #[test]
    fn needs_init_true_until_pg_version_exists() {
        let tmp = std::env::temp_dir().join(format!("lara-pg-{}", std::process::id()));
        let p = LaraluxPaths::new(tmp.clone());
        let svc = PostgresService::new();
        assert!(svc.needs_init(&p));
        std::fs::create_dir_all(p.data().join("postgres")).unwrap();
        std::fs::write(p.data().join("postgres").join("PG_VERSION"), b"16\n").unwrap();
        assert!(!svc.needs_init(&p));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn initdb_args_use_trust_and_postgres_superuser() {
        let p = LaraluxPaths::new("/tmp/lara".into());
        let svc = PostgresService::new();
        let args = svc.initdb_args(&p);
        assert!(args.contains(&"--username=postgres".to_string()));
        assert!(args.contains(&"--auth=trust".to_string()));
        assert!(args.iter().any(|a| a == "-D"));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p laralux-core service::postgres 2>&1 | tail -15`
Expected: FAIL — `PostgresService` not found.

- [ ] **Step 4: Implement** — insert (above the `#[cfg(test)]`) in `core/src/service/postgres.rs`:

```rust
pub struct PostgresService {
    port: u16,
}

impl PostgresService {
    pub fn new() -> Self {
        Self { port: 5432 }
    }
    fn datadir(&self, paths: &LaraluxPaths) -> PathBuf {
        paths.data().join("postgres")
    }
    fn initdb_args(&self, paths: &LaraluxPaths) -> Vec<String> {
        vec![
            "-D".to_string(),
            self.datadir(paths).display().to_string(),
            "--username=postgres".to_string(),
            "--auth=trust".to_string(),
            "--encoding=UTF8".to_string(),
        ]
    }
}

impl Default for PostgresService {
    fn default() -> Self {
        Self::new()
    }
}

impl Service for PostgresService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Postgres
    }
    fn name(&self) -> &str {
        "postgres"
    }
    fn write_config(&self, paths: &LaraluxPaths) -> Result<(), ServiceError> {
        // PostgreSQL keeps its config inside the datadir (generated by initdb);
        // runtime overrides are passed on the command line. Just ensure dirs exist.
        std::fs::create_dir_all(self.datadir(paths))?;
        std::fs::create_dir_all(paths.log())?;
        std::fs::create_dir_all(paths.tmp())?;
        Ok(())
    }
    fn needs_init(&self, paths: &LaraluxPaths) -> bool {
        !self.datadir(paths).join("PG_VERSION").is_file()
    }
    fn init(&self, paths: &LaraluxPaths) -> Result<(), ServiceError> {
        self.write_config(paths)?;
        let tool = crate::bin::resolve_bin("initdb", &crate::layout::managed_bin_dirs(paths))
            .ok_or_else(|| ServiceError::Init("initdb not found".into()))?;
        let status = std::process::Command::new(&tool)
            .args(self.initdb_args(paths))
            .status()
            .map_err(|e| ServiceError::Init(format!("initdb: {e}")))?;
        if !status.success() {
            return Err(ServiceError::Init("initdb failed".into()));
        }
        Ok(())
    }
    fn command(&self, paths: &LaraluxPaths) -> SpawnSpec {
        SpawnSpec::new("postgres")
            .arg("-D")
            .arg(self.datadir(paths).display().to_string())
            .arg("-p")
            .arg(self.port.to_string())
            .arg("-k")
            .arg(paths.tmp().display().to_string())
            .arg("-c")
            .arg("listen_addresses=127.0.0.1")
            .arg("-c")
            .arg("logging_collector=on")
            .arg("-c")
            .arg(format!("log_directory={}", paths.log().display()))
            .arg("-c")
            .arg("log_filename=postgres.log")
    }
    fn health_check(&self, _paths: &LaraluxPaths) -> Result<(), ServiceError> {
        probe_tcp(self.port)
    }
    fn pre_start(&self, paths: &LaraluxPaths) -> Result<(), ServiceError> {
        // Clear a stale postmaster + unix socket from a previous run.
        crate::service::cleanup_stale_endpoint(
            Some(&self.datadir(paths).join("postmaster.pid")),
            Some(&paths.tmp().join(".s.PGSQL.5432")),
        );
        Ok(())
    }
}
```

- [ ] **Step 5: Declare the module** — in `core/src/service/mod.rs`, add (with the other `pub mod` lines, after `pub mod php_fpm;`):

```rust
pub mod postgres;
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p laralux-core service::postgres 2>&1 | tail -15`
Expected: PASS — the three tests.

- [ ] **Step 7: Commit**

```bash
git add core/src/service/postgres.rs core/src/service/mod.rs
git commit -m "feat(core): PostgresService (stateful, initdb trust auth, 5432)"
```

---

### Task 3: Core wiring — ManagedTool, config flag, registry

**Files:**
- Modify: `core/src/tools.rs` (enum + ALL + info + available_versions + install_version)
- Modify: `core/src/config.rs` (`ServicesConfig.postgres`)
- Modify: `core/src/service/registry.rs` (include `PostgresService`)
- Modify: `core/src/lib.rs` (export `ServicesConfig`)

**Interfaces:**
- Consumes: Task 1 `install_postgres_version` + `KNOWN_POSTGRES_VERSIONS`; Task 2 `ServiceKind::Postgres`, `PostgresService`.
- Produces: `ManagedTool::Postgres`; `config.services.postgres: bool` (default false); registry registers Postgres when enabled.

- [ ] **Step 1: Write failing tests** — append to `core/src/service/registry.rs`'s `mod tests`:

```rust
    #[test]
    fn postgres_included_only_when_enabled() {
        let p = LaraluxPaths::new("/tmp/lara".into());
        let mut cfg = Config::default();
        assert!(!build_services(&cfg, &p).iter().any(|s| s.kind() == ServiceKind::Postgres),
            "postgres must be opt-in (off by default)");
        cfg.services.postgres = true;
        assert!(build_services(&cfg, &p).iter().any(|s| s.kind() == ServiceKind::Postgres));
    }
```

And append to `core/src/tools.rs`'s `mod tests`:

```rust
    #[test]
    fn postgres_tool_info_and_versions() {
        assert_eq!(key(ManagedTool::Postgres), "postgres");
        assert_eq!(info(ManagedTool::Postgres).service_kind, Some(ServiceKind::Postgres));
        let paths = LaraluxPaths::new(std::env::temp_dir().join(format!("lara-pgtool-{}", std::process::id())));
        let vs = available_versions(ManagedTool::Postgres, &paths);
        assert_eq!(vs.len(), crate::postgres_static::KNOWN_POSTGRES_VERSIONS.len());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p laralux-core registry:: tools:: 2>&1 | tail -20`
Expected: FAIL — no `ManagedTool::Postgres` / `config.services.postgres`.

- [ ] **Step 3: Add the config flag** — in `core/src/config.rs`, add `postgres` to `ServicesConfig` and its default. Replace:

```rust
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
```

with:

```rust
pub struct ServicesConfig {
    pub nginx: bool,
    pub php: bool,
    pub mariadb: bool,
    pub redis: bool,
    pub mailpit: bool,
    #[serde(default)]
    pub postgres: bool,
}

impl Default for ServicesConfig {
    fn default() -> Self {
        Self { nginx: true, php: true, mariadb: true, redis: true, mailpit: true, postgres: false }
    }
}
```

- [ ] **Step 4: Add the ManagedTool variant** — in `core/src/tools.rs`:

(a) Replace the enum + ALL:

```rust
pub enum ManagedTool { Php, Nginx, Mariadb, Postgres, Redis, Mailpit, Mkcert, Composer, Node }

impl ManagedTool {
    pub const ALL: [ManagedTool; 9] = [
        ManagedTool::Php, ManagedTool::Nginx, ManagedTool::Mariadb, ManagedTool::Postgres,
        ManagedTool::Redis, ManagedTool::Mailpit, ManagedTool::Mkcert, ManagedTool::Composer,
        ManagedTool::Node,
    ];
}
```

(b) In `info()`'s match (after the `Mariadb =>` arm), add:

```rust
        Postgres => ToolInfo { key: "postgres", display: "PostgreSQL", cli_binaries: &["psql", "pg_dump", "pg_restore", "createdb", "dropdb"], service_kind: Some(ServiceKind::Postgres) },
```

(c) In `available_versions()`'s match (after the `Mariadb =>` arm), add:

```rust
        ManagedTool::Postgres => known_catalog(
            &crate::postgres_static::KNOWN_POSTGRES_VERSIONS,
            crate::layout::installed_versions(paths, "postgres"),
            &cfg.versions.get("postgres").cloned().unwrap_or_default(),
        ),
```

(d) In `install_version()`'s match (after the `Mariadb =>` arm), add:

```rust
        ManagedTool::Postgres => crate::postgres_static::install_postgres_version(paths, version, downloader, runner, sink)
            .map_err(|e| ToolError::Install(e.to_string())),
```

- [ ] **Step 5: Register the service** — in `core/src/service/registry.rs`, add the import and the conditional. Add near the other `use` lines:

```rust
use crate::service::postgres::PostgresService;
```

and inside `build_services`, after the mariadb block:

```rust
    if config.services.postgres {
        services.push(Box::new(PostgresService::new()));
    }
```

- [ ] **Step 6: Export `ServicesConfig`** — in `core/src/lib.rs`, change `pub use config::Config;` to:

```rust
pub use config::{Config, ServicesConfig};
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p laralux-core 2>&1 | grep "test result:"`
Expected: all green (including the two new tests).

- [ ] **Step 8: Commit**

```bash
git add core/src/tools.rs core/src/config.rs core/src/service/registry.rs core/src/lib.rs
git commit -m "feat(core): wire PostgreSQL tool + opt-in config flag + registry"
```

---

### Task 4: Core — `Orchestrator::reconcile`

**Files:**
- Modify: `core/src/orchestrator.rs` (add `reconcile`; test)

**Interfaces:**
- Consumes: existing `stop`, `services`, `states`, `start`.
- Produces: `pub fn reconcile(&mut self, new_services: Vec<Box<dyn Service>>)`.

- [ ] **Step 1: Write the failing test** — append to `core/src/orchestrator.rs`'s `mod tests`:

```rust
    #[test]
    fn reconcile_adds_and_removes_services() {
        let spawner = FakeSpawner::new();
        let mut o = Orchestrator::new(
            LaraluxPaths::new("/tmp/lara".into()),
            vec![Box::new(Dummy { kind: ServiceKind::Redis, name: "redis-server" })],
            Box::new(spawner),
        );
        o.start(ServiceKind::Redis).unwrap();
        assert_eq!(o.state(ServiceKind::Redis), ServiceState::Running);

        // Reconcile to a set WITHOUT redis but WITH mariadb: redis is stopped+dropped.
        o.reconcile(vec![Box::new(Dummy { kind: ServiceKind::Mariadb, name: "mariadbd" })]);
        assert_eq!(o.state(ServiceKind::Redis), ServiceState::Stopped);
        assert!(o.start_order().contains(&ServiceKind::Mariadb));
        assert!(!o.start_order().contains(&ServiceKind::Redis));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p laralux-core orchestrator::tests::reconcile 2>&1 | tail -12`
Expected: FAIL — no method `reconcile`.

- [ ] **Step 3: Implement** — add this method to `impl Orchestrator` (near `reap_orphans`/`start_all`):

```rust
    /// Replace the managed service definitions with `new_services`. Any service
    /// kind present before but absent now is stopped (terminating its child and
    /// dropping its handle) and its state cleared; running handles for surviving
    /// kinds are preserved. Used to apply a `config.services` change at runtime
    /// without restarting the app or orphaning processes.
    pub fn reconcile(&mut self, new_services: Vec<Box<dyn Service>>) {
        let new_kinds: std::collections::HashSet<ServiceKind> =
            new_services.iter().map(|s| s.kind()).collect();
        let removed: Vec<ServiceKind> = self
            .services
            .iter()
            .map(|s| s.kind())
            .filter(|k| !new_kinds.contains(k))
            .collect();
        for kind in removed {
            let _ = self.stop(kind);
            self.states.remove(&kind);
        }
        self.services = new_services;
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p laralux-core orchestrator::tests::reconcile 2>&1 | tail -8`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/orchestrator.rs
git commit -m "feat(core): Orchestrator::reconcile for runtime service enable/disable"
```

---

### Task 5: Desktop — `set_service_enabled` + `service_flags` commands

**Files:**
- Modify: `src-tauri/src/commands.rs` (two commands)
- Modify: `src-tauri/src/main.rs` (register both)

**Interfaces:**
- Consumes: `laralux_core::{build_services, ServicesConfig, ServiceKind, Config}`, `AppState`, `state.orch`, `state.paths`, `lock_err`.
- Produces: `set_service_enabled(kind: ServiceKind, enabled: bool) -> Result<Vec<ServiceStatus>, String>`, `service_flags() -> Result<ServicesConfig, String>`.

- [ ] **Step 1: Implement the commands** — add to `src-tauri/src/commands.rs` (e.g. after `stack_stop_all`):

```rust
/// Current per-service enable flags (drives the Settings "Services" toggles).
#[tauri::command]
pub fn service_flags(state: tauri::State<AppState>) -> Result<laralux_core::ServicesConfig, String> {
    let config = Config::load(&state.paths.config_file()).unwrap_or_default();
    Ok(config.services)
}

/// Enable/disable a service: persist the flag, then reconcile the orchestrator so
/// the change takes effect immediately (a disabled service is stopped).
#[tauri::command]
pub fn set_service_enabled(
    state: tauri::State<AppState>,
    kind: ServiceKind,
    enabled: bool,
) -> Result<Vec<ServiceStatus>, String> {
    let mut config = Config::load(&state.paths.config_file()).unwrap_or_default();
    match kind {
        ServiceKind::Nginx => config.services.nginx = enabled,
        ServiceKind::PhpFpm => config.services.php = enabled,
        ServiceKind::Mariadb => config.services.mariadb = enabled,
        ServiceKind::Postgres => config.services.postgres = enabled,
        ServiceKind::Redis => config.services.redis = enabled,
        ServiceKind::Mailpit => config.services.mailpit = enabled,
        ServiceKind::Coredns => return Err("coredns is managed automatically".into()),
    }
    config.save(&state.paths.config_file()).map_err(|e| e.to_string())?;
    let new_services = build_services(&config, &state.paths);
    let mut orch = state.orch.lock().map_err(lock_err)?;
    orch.reconcile(new_services);
    orch.refresh();
    Ok(orch.snapshot())
}
```

- [ ] **Step 2: Register the commands** — in `src-tauri/src/main.rs` `invoke_handler`, add after `commands::delete_site_folder,`:

```rust
            commands::service_flags,
            commands::set_service_enabled,
```

- [ ] **Step 3: Build to verify**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo build -p laralux-desktop 2>&1 | tail -4`
Expected: `Finished`, no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(desktop): service_flags + set_service_enabled commands"
```

---

### Task 6: Frontend — IPC/state/constants + Settings "Services" toggle

**Files:**
- Modify: `src/ipc/types.ts` (ServicesConfig type), `src/ipc/commands.ts` (two wrappers)
- Modify: `src/state.ts` (`Postgres` in services; `serviceFlags`)
- Modify: `src/ui/constants.ts` (display order + Postgres display)
- Modify: `src/ui/render.ts` (load flags into state)
- Modify: `src/ui/views/settings.ts` (Services section)
- Modify: `src/ui/events.ts` (toggle action)

**Interfaces:**
- Consumes: Task 5 `service_flags`, `set_service_enabled`.
- Produces: `state.serviceFlags: Record<string, boolean>`; `serviceFlags()`, `setServiceEnabled(kind, enabled)`; `SVC_ORDER` + `FLAG_KEY` constants; `toggleServiceEnabled(kind)` in settings.

- [ ] **Step 1: Types + IPC** — in `src/ipc/types.ts` add:

```ts
export interface ServicesFlags {
  nginx: boolean; php: boolean; mariadb: boolean; redis: boolean; mailpit: boolean; postgres: boolean;
}
```

In `src/ipc/commands.ts` add:

```ts
import type { ServicesFlags } from "./types";

/** Current per-service enable flags. */
export const serviceFlags = (): Promise<ServicesFlags> => invoke<ServicesFlags>("service_flags");

/** Enable/disable a service by ServiceKind ("Nginx" | "PhpFpm" | ... | "Postgres"). */
export const setServiceEnabled = (kind: string, enabled: boolean): Promise<unknown> =>
  invoke("set_service_enabled", { kind, enabled });
```

(If `commands.ts` already imports from `./types`, merge the import instead of adding a second line.)

- [ ] **Step 2: State** — in `src/state.ts`:

Add `Postgres: "Stopped"` to the initial `services` object (so `applyServices` accepts it):

```ts
  services: { Nginx: "Stopped", PhpFpm: "Stopped", Mariadb: "Stopped", Postgres: "Stopped", Redis: "Stopped", Mailpit: "Stopped" },
```

Add a field to the `AppState` interface (near `services:`):

```ts
  serviceFlags: Record<string, boolean>;
```

and to the initial state object:

```ts
  serviceFlags: { nginx: true, php: true, mariadb: true, redis: true, mailpit: true, postgres: false },
```

- [ ] **Step 3: Constants** — in `src/ui/constants.ts`:

Add `Postgres` to `DISP`:

```ts
export const DISP: Record<string, string> = {
  Nginx: "Nginx", PhpFpm: "PHP-FPM", Mariadb: "MariaDB", Postgres: "PostgreSQL", Redis: "Redis", Mailpit: "Mailpit",
};
```

Append a stack display order and the ServiceKind→config-flag-key map:

```ts
// Service display order for the dashboard grid + Settings toggles.
export const SVC_ORDER = ["Nginx", "PhpFpm", "Mariadb", "Postgres", "Redis", "Mailpit"];

// ServiceKind (enum variant) -> ServicesFlags key.
export const FLAG_KEY: Record<string, string> = {
  Nginx: "nginx", PhpFpm: "php", Mariadb: "mariadb", Postgres: "postgres", Redis: "redis", Mailpit: "mailpit",
};
```

- [ ] **Step 4: Load flags into state on boot** — in `src/ui/render.ts`, near where the initial snapshot is applied (the boot section that calls `applyServices`), also fetch flags. Add an exported helper and call it from the boot path. Add:

```ts
import { serviceFlags } from "../ipc/commands";

export async function loadServiceFlags(): Promise<void> {
  try {
    const f = await serviceFlags();
    if (f && typeof f === "object") {
      state.serviceFlags = f as unknown as Record<string, boolean>;
    }
  } catch {
    /* keep defaults */
  }
}
```

Then in `src/main.ts` boot sequence (where `stack_status`/`applyServices` runs at startup), call `await loadServiceFlags()` before the first `render()`. (Find the existing boot block that imports from `./ui/render` and awaits initial data; add the call there.)

- [ ] **Step 5: Settings "Services" section** — in `src/ui/views/settings.ts`, import and render the toggles. Replace the file's imports/top with:

```ts
import { state } from "../../state";
import { render } from "../render";
import { SVC_ORDER, DISP, FLAG_KEY } from "../constants";
import { setServiceEnabled, serviceFlags } from "../../ipc/commands";
```

Inside `settingsView()`, before the final `"</div>"` of the settings-card (after the "Start on login" row), insert a Services block. Each row has a **Manage** button (`data-action="open-tool"` — already wired in events.ts) that opens the existing tool modal to install / pick a version; this is the install entry point for opt-in PostgreSQL (and version management for the rest). `FLAG_KEY[k]` is also the managed-tool key:

```ts
    '<div class="set-row"><div class="grow"><div class="t">Services</div><div class="h">Enable/disable services in the stack</div></div></div>' +
    SVC_ORDER.map((k) => {
      const on = !!state.serviceFlags[FLAG_KEY[k]];
      return '<div class="set-row sub"><div class="grow"><div class="t">' + DISP[k] + "</div></div>" +
        '<button class="btn-xs" data-action="open-tool" data-tool="' + FLAG_KEY[k] + '">Manage</button>' +
        '<button class="' + (on ? "toggle-on" : "toggle-off") + '" data-action="svc-enable" data-kind="' + k + '" aria-pressed="' + on + '"><span class="knob"></span></button></div>';
    }).join("") +
```

Add the handler at the bottom of `settings.ts`:

```ts
export async function toggleServiceEnabled(kind: string): Promise<void> {
  const flagKey = FLAG_KEY[kind];
  const next = !state.serviceFlags[flagKey];
  state.serviceFlags = { ...state.serviceFlags, [flagKey]: next };
  render();
  try {
    await setServiceEnabled(kind, next);
    // Refresh both the snapshot and the persisted flags.
    const f = await serviceFlags();
    if (f && typeof f === "object") state.serviceFlags = f as unknown as Record<string, boolean>;
  } catch (e) {
    state.serviceFlags = { ...state.serviceFlags, [flagKey]: !next };
  }
  render();
}
```

- [ ] **Step 6: Events** — in `src/ui/events.ts`:

Add `toggleServiceEnabled` to the settings import (currently `import { toggleDark } from "./views/settings";`):

```ts
import { toggleDark, toggleServiceEnabled } from "./views/settings";
```

Add a dispatch branch (next to `toggle-dark`):

```ts
    else if (a === "svc-enable") toggleServiceEnabled(el.getAttribute("data-kind")!);
```

- [ ] **Step 7: Styles** — append to `src/styles.css` (toggle + sub-row; reuse existing `.toggle-off/.knob` look):

```css
.set-row.sub { padding-left: 8px; }
.toggle-on, .toggle-off { width: 40px; height: 22px; border-radius: 999px; border: none; position: relative; cursor: pointer; }
.toggle-on { background: var(--accent, #16a34a); }
.toggle-off { background: var(--border, #cbd5e1); }
.toggle-on .knob, .toggle-off .knob { position: absolute; top: 2px; width: 18px; height: 18px; border-radius: 50%; background: #fff; transition: left .12s; }
.toggle-off .knob { left: 2px; }
.toggle-on .knob { left: 20px; }
```

(If `.toggle-off`/`.knob` already exist in `styles.css`, keep the existing rules and add only `.toggle-on`, `.toggle-on .knob`, and `.set-row.sub`.)

- [ ] **Step 8: Build to verify**

Run: `npm run build 2>&1 | tail -8`
Expected: `✓ built`, no TypeScript errors.

- [ ] **Step 9: Commit**

```bash
git add src/ipc/types.ts src/ipc/commands.ts src/state.ts src/ui/constants.ts src/ui/render.ts src/main.ts src/ui/views/settings.ts src/ui/events.ts src/styles.css
git commit -m "feat(ui): Services enable/disable toggles in Settings"
```

---

### Task 7: Frontend — dashboard renders the enabled service set (+ PostgreSQL card)

**Files:**
- Modify: `src/ui/views/dashboard.ts` (dynamic grid + count + Postgres display data + install entry)
- Modify: `src/ui/render.ts` (`runningCount` over the enabled set)

**Interfaces:**
- Consumes: `state.serviceFlags`, `SVC_ORDER`, `FLAG_KEY`, `DISP`, `state.services`.
- Produces: dashboard shows a card per enabled service; running count is `X / <enabled count>`; an enabled-but-not-installed PostgreSQL card offers install via the existing `open-tool` action.

- [ ] **Step 1: Add Postgres display data** — in `src/ui/views/dashboard.ts`, extend the maps near the top:

```ts
const SVC_ICON: Record<string, string> = { Nginx: I.svcNginx, PhpFpm: I.svcPhp, Mariadb: I.svcMaria, Postgres: I.svcMaria, Redis: I.svcRedis, Mailpit: I.svcMail };
const PORTS: Record<string, string[]> = { Nginx: ["80", "443"], PhpFpm: ["socket"], Mariadb: ["3306"], Postgres: ["5432"], Redis: ["6379"], Mailpit: ["8025", "1025"] };
const LOG_FILE: Record<string, string> = { Nginx: "nginx-error.log", PhpFpm: "php-fpm.log", Mariadb: "mariadb.log", Postgres: "postgres.log", Redis: "redis.log", Mailpit: "mailpit.log" };
```

- [ ] **Step 2: Compute the enabled set + dynamic grid** — in `src/ui/views/dashboard.ts`, add the import:

```ts
import { SVC_ORDER, FLAG_KEY, DISP, META } from "../constants";
```

(Replace the existing `from "../constants"` import. **Drop `SVC_KINDS`** — after this task the dashboard no longer references it; leaving it imported trips `noUnusedLocals`.) Add a helper and use it for the grid + dots:

```ts
// Services currently enabled in the stack, in display order.
function enabledKinds(): string[] {
  return SVC_ORDER.filter((k) => state.serviceFlags[FLAG_KEY[k]]);
}
```

Replace the `dashboard()` body's grid/summary derivations:
- `const run = runningCount();` stays.
- `const allRunning = run === 5;` → `const kinds = enabledKinds(); const allRunning = run === kinds.length && kinds.length > 0;`
- the dots: `const dots = SVC_KINDS.map(...)` → `const dots = kinds.map(...)`.
- the cards: `const cards = SVC_KINDS.map(serviceCard).join("");` → `const cards = kinds.map(serviceCard).join("");`
- the summary numerator/denominator: `'<span class="den">/ 5</span>'` → `'<span class="den">/ ' + kinds.length + '</span>'`.

- [ ] **Step 3: `runningCount` over the enabled set** — in `src/ui/render.ts`, change `runningCount` to count only enabled services:

```ts
export function runningCount(): number {
  return SVC_ORDER.filter((k) => state.serviceFlags[FLAG_KEY[k]] && state.services[k] === "Running").length;
}
```

and update its import line to pull `SVC_ORDER, FLAG_KEY` from `../ui/constants` (merge with the existing constants import in render.ts; it currently imports `SVC_KINDS, COMP_ORDER, ...`).

- [ ] **Step 4: Build to verify**

Run: `npm run build 2>&1 | tail -8`
Expected: `✓ built`, no TS errors (no dangling `SVC_KINDS` references left unimported).

- [ ] **Step 5: Manual check of the rendered grid (read-only)**

Run: `grep -n "enabledKinds\|kinds.length\|kinds.map" src/ui/views/dashboard.ts`
Expected: the grid, dots, and denominator all derive from `kinds` (the enabled set), not a hardcoded 5.

- [ ] **Step 6: Commit**

```bash
git add src/ui/views/dashboard.ts src/ui/render.ts
git commit -m "feat(ui): dashboard renders the enabled service set (PostgreSQL card)"
```

---

## Final verification (after all tasks)

- [ ] `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p laralux-core 2>&1 | grep "test result:"` — all green.
- [ ] `cargo build -p laralux-desktop 2>&1 | tail -3` — Finished.
- [ ] `npm run build 2>&1 | tail -5` — built.
- [ ] Manual smoke (`npm run build && cargo run -p laralux-desktop`): Settings → Services → enable PostgreSQL → a PostgreSQL card appears on the Dashboard; install it (Setup/tool modal, pick version) → Start → DbGate connects to `127.0.0.1:5432` (user `postgres`, empty password) and lists databases; disable PostgreSQL → card disappears and the process stops; MariaDB and the other four still behave as before.
