# MongoDB Service Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add MongoDB as an opt-in (off by default), no-apt, multi-version managed service installed as an official static tarball into `~/laralux/`, orchestrated like the other DB services, reachable from DbGate, with `mongosh` bundled as a CLI.

**Architecture:** Clone the just-shipped PostgreSQL sub-project. One installer (`mongodb_static.rs`), one service (`service/mongodb.rs`), and the tool/config/registry/command/frontend wiring. The service enable/disable foundation (`ServicesConfig` flags, `Orchestrator::reconcile`, `set_service_enabled`, Settings→Services toggles, dashboard `enabledKinds()`) already exists from PostgreSQL and is reused with no new mechanism.

**Tech Stack:** Rust (laralux-core: zero Tauri deps; laralux-desktop: Tauri 2), TypeScript (Vite strict, `noUnusedLocals`/`noUnusedParameters`).

## Global Constraints

- **laralux-core keeps ZERO Tauri dependencies.** No new crate dependency needed (both MongoDB artifacts are plain gzip tarballs — `tar -xzf`, no `zip`).
- **MongoDB is opt-in: `ServicesConfig.mongodb` defaults to `false`** with `#[serde(default)]` so older config files load.
- **Verified download URLs (HTTP 200, do not alter):**
  - Server: `https://fastdl.mongodb.org/linux/mongodb-linux-{arch}-ubuntu2204-{version}.tgz`, `{arch}` ∈ {`x86_64`,`aarch64`}.
  - Shell: `https://downloads.mongodb.com/compass/mongosh-{version}-linux-{arch2}.tgz`, `{arch2}` ∈ {`x64`,`arm64`}.
- **Pinned versions:** `MONGODB_VERSION = "8.0.4"`, `KNOWN_MONGODB_VERSIONS = ["8.0.4", "7.0.15"]`, `MONGOSH_VERSION = "2.3.8"`.
- **mongosh download/extract failure is NON-fatal** (server still works); the **server tarball** failing IS fatal.
- Port `27017`, bind `127.0.0.1`. No auth (standalone mongod), like the password-less MariaDB/Postgres defaults.
- Commits: **no `Co-Authored-By` trailer.** Work on `master` (direct commits, this session's convention).
- Follow existing patterns exactly: `postgres_static.rs`, `service/postgres.rs` are the reference implementations to mirror.

---

### Task 1: MongoDB installer (`core/src/mongodb_static.rs`)

**Files:**
- Create: `core/src/mongodb_static.rs`
- Modify: `core/src/lib.rs` (add `pub mod mongodb_static;` near line 31 after `postgres_static`; add `pub use` near line 73)

**Interfaces:**
- Consumes: `crate::paths::LaraluxPaths`, `crate::progress::ProgressSink`, `crate::scaffold::CommandRunner`, `crate::setup::Downloader`, `crate::layout::set_current`.
- Produces: `MONGODB_VERSION`, `KNOWN_MONGODB_VERSIONS`, `MONGOSH_VERSION`, `mongodb_arch()`, `mongosh_arch()`, `mongodb_url(version, arch)`, `mongosh_url(version, arch2)`, `install_mongodb(paths, downloader, runner, sink) -> Result<String, MongodbError>`, `install_mongodb_version(paths, version, downloader, runner, sink) -> Result<String, MongodbError>`, `MongodbError`.

- [ ] **Step 1: Write the installer file**

Create `core/src/mongodb_static.rs` with exactly this content:

```rust
use crate::paths::LaraluxPaths;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

pub const MONGODB_VERSION: &str = "8.0.4";

/// Curated MongoDB server versions offered in the Setup/version modal. Both
/// verified present on fastdl.mongodb.org as `ubuntu2204` static tarballs.
pub const KNOWN_MONGODB_VERSIONS: [&str; 2] = ["8.0.4", "7.0.15"];

/// Bundled `mongosh` shell version (Apache-2.0), pinned independently of the
/// server (MongoDB distributes the shell on its own cadence).
pub const MONGOSH_VERSION: &str = "2.3.8";

#[derive(Debug, thiserror::Error)]
pub enum MongodbError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Server tarball arch token.
pub fn mongodb_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("aarch64"), _ => None }
}

/// `mongosh` tarball arch token (different naming than the server).
pub fn mongosh_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("x64"), "aarch64" => Some("arm64"), _ => None }
}

pub fn mongodb_url(version: &str, arch: &str) -> String {
    format!("https://fastdl.mongodb.org/linux/mongodb-linux-{arch}-ubuntu2204-{version}.tgz")
}

pub fn mongosh_url(version: &str, arch2: &str) -> String {
    format!("https://downloads.mongodb.com/compass/mongosh-{version}-linux-{arch2}.tgz")
}

/// Make `link` (under basedir) a relative symlink to `target_rel`.
fn rel_symlink(basedir: &std::path::Path, link_name: &str, target_rel: &str) -> std::io::Result<()> {
    let link = basedir.join(link_name);
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    { std::os::unix::fs::symlink(target_rel, &link)?; }
    Ok(())
}

/// Download + install the default (pinned) MongoDB version.
pub fn install_mongodb(
    paths: &LaraluxPaths, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, MongodbError> {
    install_mongodb_version(paths, MONGODB_VERSION, downloader, runner, sink)
}

/// Download the server tarball + the `mongosh` tarball (both plain gzip),
/// flatten each (`--strip-components=1`) into bin/mongodb/<version>/, and create
/// top-level symlinks to the executables. Idempotent. The server tarball is
/// required; a `mongosh` failure is logged and tolerated.
pub fn install_mongodb_version(
    paths: &LaraluxPaths, version: &str, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, MongodbError> {
    let basedir = paths.version_dir("mongodb", version);
    if basedir.join("bin").join("mongod").exists() {
        let _ = crate::layout::set_current(paths, "mongodb", version);
        return Ok(version.to_string());
    }
    let arch = mongodb_arch().ok_or_else(|| MongodbError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    let _ = std::fs::remove_dir_all(&basedir);
    std::fs::create_dir_all(&basedir)?;

    // Server tarball (required).
    let server = paths.tmp().join("mongodb.tgz");
    downloader.fetch_with_progress(&mongodb_url(version, arch), &server, sink)
        .map_err(|e| MongodbError::Download(e.to_string()))?;
    runner.run("tar", &[
        "-xzf".into(), server.display().to_string(),
        "--strip-components=1".into(),
        "-C".into(), basedir.display().to_string(),
    ], None).map_err(|e| MongodbError::Extract(e.to_string()))?;
    if !basedir.join("bin").join("mongod").exists() {
        return Err(MongodbError::Extract("mongod binary missing after extract".into()));
    }

    // mongosh shell (best-effort: server is usable without it).
    if let Some(arch2) = mongosh_arch() {
        let shell = paths.tmp().join("mongosh.tgz");
        let ok = downloader.fetch_with_progress(&mongosh_url(MONGOSH_VERSION, arch2), &shell, sink).is_ok()
            && runner.run("tar", &[
                "-xzf".into(), shell.display().to_string(),
                "--strip-components=1".into(),
                "-C".into(), basedir.display().to_string(),
            ], None).is_ok();
        if !ok {
            eprintln!("laralux: mongosh install skipped (download/extract failed); server is usable");
        }
        let _ = std::fs::remove_file(&shell);
    }

    // Top-level symlinks so bin/mongodb/current/<exe> resolves.
    for exe in ["mongod", "mongos", "mongosh"] {
        if basedir.join("bin").join(exe).exists() {
            let _ = rel_symlink(&basedir, exe, &format!("bin/{exe}"));
        }
    }
    let _ = std::fs::remove_file(&server);
    crate::layout::set_current(paths, "mongodb", version)
        .map_err(|e| MongodbError::Extract(e.to_string()))?;
    Ok(version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_arch() {
        assert_eq!(
            mongodb_url("8.0.4", "x86_64"),
            "https://fastdl.mongodb.org/linux/mongodb-linux-x86_64-ubuntu2204-8.0.4.tgz"
        );
        assert_eq!(
            mongodb_url("8.0.4", "aarch64"),
            "https://fastdl.mongodb.org/linux/mongodb-linux-aarch64-ubuntu2204-8.0.4.tgz"
        );
        assert_eq!(
            mongosh_url("2.3.8", "x64"),
            "https://downloads.mongodb.com/compass/mongosh-2.3.8-linux-x64.tgz"
        );
        assert_eq!(
            mongosh_url("2.3.8", "arm64"),
            "https://downloads.mongodb.com/compass/mongosh-2.3.8-linux-arm64.tgz"
        );
        assert_eq!(
            mongodb_arch(),
            match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("aarch64"), _ => None }
        );
        assert_eq!(
            mongosh_arch(),
            match std::env::consts::ARCH { "x86_64" => Some("x64"), "aarch64" => Some("arm64"), _ => None }
        );
    }

    #[test]
    fn known_versions_include_pinned_default() {
        assert!(KNOWN_MONGODB_VERSIONS.contains(&MONGODB_VERSION));
    }
}
```

- [ ] **Step 2: Register the module in `core/src/lib.rs`**

After the line `pub mod postgres_static;` (≈ line 31) add:

```rust
pub mod mongodb_static;
```

After the line `pub use postgres_static::{install_postgres, install_postgres_version, PostgresError};` (≈ line 73) add:

```rust
pub use mongodb_static::{install_mongodb, install_mongodb_version, MongodbError};
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p laralux-core mongodb_static`
Expected: PASS (`urls_and_arch`, `known_versions_include_pinned_default`).

- [ ] **Step 4: Commit**

```bash
git add core/src/mongodb_static.rs core/src/lib.rs
git commit -m "feat(core): mongodb_static installer (server tgz + bundled mongosh)"
```

---

### Task 2: MongoDB service (`core/src/service/mongodb.rs`)

**Files:**
- Create: `core/src/service/mongodb.rs`
- Modify: `core/src/service/mod.rs` (add `Mongodb` to `ServiceKind` enum ≈ line 10; add `pub mod mongodb;` ≈ line 133, alphabetical with the other `pub mod` lines)

**Interfaces:**
- Consumes: `crate::paths::LaraluxPaths`, `crate::service::{probe_tcp, Service, ServiceError, ServiceKind, SpawnSpec, cleanup_stale_endpoint}`.
- Produces: `ServiceKind::Mongodb`, `MongodbService` (with `pub fn new() -> Self`).

> Note: adding `ServiceKind::Mongodb` makes the exhaustive `match kind` in `src-tauri/src/commands.rs::set_service_enabled` non-exhaustive — that break is in the **desktop** crate and is fixed in Task 4. This task's verification is `cargo test -p laralux-core`, which compiles core only and stays green.

- [ ] **Step 1: Add the `Mongodb` variant to `ServiceKind`**

In `core/src/service/mod.rs`, change the enum (currently `Nginx, PhpFpm, Mariadb, Postgres, Redis, Mailpit, Coredns`) to insert `Mongodb` after `Postgres`:

```rust
pub enum ServiceKind {
    Nginx,
    PhpFpm,
    Mariadb,
    Postgres,
    Mongodb,
    Redis,
    Mailpit,
    Coredns,
}
```

- [ ] **Step 2: Register the module**

In `core/src/service/mod.rs`, add alongside the other `pub mod` lines (after `pub mod mariadb;`, keeping alphabetical order):

```rust
pub mod mongodb;
```

- [ ] **Step 3: Write the failing test (service file with tests)**

Create `core/src/service/mongodb.rs` with exactly this content:

```rust
use crate::paths::LaraluxPaths;
use crate::service::{probe_tcp, Service, ServiceError, ServiceKind, SpawnSpec};
use std::path::PathBuf;

pub struct MongodbService {
    port: u16,
}

impl MongodbService {
    pub fn new() -> Self {
        Self { port: 27017 }
    }
    fn dbpath(&self, paths: &LaraluxPaths) -> PathBuf {
        paths.data().join("mongodb")
    }
}

impl Default for MongodbService {
    fn default() -> Self {
        Self::new()
    }
}

impl Service for MongodbService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Mongodb
    }
    fn name(&self) -> &str {
        "mongodb"
    }
    fn write_config(&self, paths: &LaraluxPaths) -> Result<(), ServiceError> {
        // mongod requires the dbpath dir to pre-exist; it populates the
        // WiredTiger files itself on first start (no separate init step).
        std::fs::create_dir_all(self.dbpath(paths))?;
        std::fs::create_dir_all(paths.log())?;
        std::fs::create_dir_all(paths.tmp())?;
        Ok(())
    }
    fn command(&self, paths: &LaraluxPaths) -> SpawnSpec {
        SpawnSpec::new("mongod")
            .arg("--dbpath")
            .arg(self.dbpath(paths).display().to_string())
            .arg("--port")
            .arg(self.port.to_string())
            .arg("--bind_ip")
            .arg("127.0.0.1")
            .arg("--unixSocketPrefix")
            .arg(paths.tmp().display().to_string())
            .arg("--logpath")
            .arg(paths.log().join("mongodb.log").display().to_string())
            .arg("--logappend")
    }
    fn health_check(&self, _paths: &LaraluxPaths) -> Result<(), ServiceError> {
        probe_tcp(self.port)
    }
    fn pre_start(&self, paths: &LaraluxPaths) -> Result<(), ServiceError> {
        // Clear a stale lock + unix socket from a previous run.
        crate::service::cleanup_stale_endpoint(
            None,
            Some(&paths.tmp().join("mongodb-27017.sock")),
        );
        let _ = std::fs::remove_file(self.dbpath(paths).join("mongod.lock"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Service, ServiceKind};

    #[test]
    fn command_and_kind() {
        let p = LaraluxPaths::new("/tmp/lara".into());
        let svc = MongodbService::new();
        let spec = svc.command(&p);
        assert_eq!(spec.program, "mongod");
        assert!(spec.args.iter().any(|a| a == "--dbpath"));
        assert!(spec.args.iter().any(|a| a == "27017"));
        assert!(spec.args.iter().any(|a| a == "127.0.0.1"));
        assert!(spec.args.iter().any(|a| a.ends_with("mongodb.log")));
        assert!(spec.args.iter().any(|a| a == "--unixSocketPrefix"));
        assert_eq!(svc.kind(), ServiceKind::Mongodb);
        assert_eq!(svc.name(), "mongodb");
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p laralux-core`
Expected: PASS (new `service::mongodb::tests::command_and_kind` plus all existing tests; core compiles despite the desktop-crate match break, which is out of this crate).

- [ ] **Step 5: Commit**

```bash
git add core/src/service/mongodb.rs core/src/service/mod.rs
git commit -m "feat(core): MongodbService (standalone mongod on 127.0.0.1:27017)"
```

---

### Task 3: Tool / config / registry wiring (`core/src/tools.rs`, `config.rs`, `service/registry.rs`)

**Files:**
- Modify: `core/src/tools.rs` (enum ≈ line 9, `ALL` ≈ line 12, `info()` ≈ line 42, `available_versions` ≈ line 124, `install_version` ≈ line 173, plus a test)
- Modify: `core/src/config.rs` (`ServicesConfig` struct ≈ line 16, `Default` ≈ line 28)
- Modify: `core/src/service/registry.rs` (import ≈ line 7, push ≈ line 21, test)

**Interfaces:**
- Consumes: `ServiceKind::Mongodb` (Task 2), `install_mongodb_version` / `KNOWN_MONGODB_VERSIONS` (Task 1), `MongodbService` (Task 2).
- Produces: `ManagedTool::Mongodb` (key `"mongodb"`), `ServicesConfig.mongodb: bool`.

- [ ] **Step 1: Add `ManagedTool::Mongodb` in `core/src/tools.rs`**

Enum (line 9) — insert `Mongodb` after `Postgres`:

```rust
pub enum ManagedTool { Php, Nginx, Mariadb, Postgres, Mongodb, Redis, Mailpit, Mkcert, Composer, Node }
```

`ALL` array — bump the count to `10` and add the variant:

```rust
    pub const ALL: [ManagedTool; 10] = [
        ManagedTool::Php, ManagedTool::Nginx, ManagedTool::Mariadb, ManagedTool::Postgres,
        ManagedTool::Mongodb, ManagedTool::Redis, ManagedTool::Mailpit, ManagedTool::Mkcert,
        ManagedTool::Composer, ManagedTool::Node,
    ];
```

In `info()` (after the `Postgres =>` arm, line 42) add:

```rust
        Mongodb => ToolInfo { key: "mongodb", display: "MongoDB", cli_binaries: &["mongod", "mongosh"], service_kind: Some(ServiceKind::Mongodb) },
```

In `available_versions` (after the `ManagedTool::Postgres =>` arm, line 124) add:

```rust
        ManagedTool::Mongodb => known_catalog(
            &crate::mongodb_static::KNOWN_MONGODB_VERSIONS,
            crate::layout::installed_versions(paths, "mongodb"),
            &cfg.versions.get("mongodb").cloned().unwrap_or_default(),
        ),
```

In `install_version` (after the `ManagedTool::Postgres =>` arm, line 173) add:

```rust
        ManagedTool::Mongodb => crate::mongodb_static::install_mongodb_version(paths, version, downloader, runner, sink)
            .map_err(|e| ToolError::Install(e.to_string())),
```

- [ ] **Step 2: Add the tool test in `core/src/tools.rs`**

After the existing `postgres_tool_info_and_versions` test, add:

```rust
    #[test]
    fn mongodb_tool_info_and_versions() {
        assert_eq!(key(ManagedTool::Mongodb), "mongodb");
        assert_eq!(info(ManagedTool::Mongodb).service_kind, Some(ServiceKind::Mongodb));
        let paths = LaraluxPaths::new("/tmp/lara".into());
        let vs = available_versions(ManagedTool::Mongodb, &paths);
        assert_eq!(vs.len(), crate::mongodb_static::KNOWN_MONGODB_VERSIONS.len());
    }
```

- [ ] **Step 3: Add the `mongodb` flag in `core/src/config.rs`**

`ServicesConfig` struct — add after the `postgres` field:

```rust
    #[serde(default)]
    pub mongodb: bool,
```

`Default for ServicesConfig` — add `mongodb: false`:

```rust
        Self { nginx: true, php: true, mariadb: true, redis: true, mailpit: true, postgres: false, mongodb: false }
```

- [ ] **Step 4: Wire the service in `core/src/service/registry.rs`**

Add the import after `use crate::service::postgres::PostgresService;` (line 7):

```rust
use crate::service::mongodb::MongodbService;
```

Add the push after the `postgres` block (line 23):

```rust
    if config.services.mongodb {
        services.push(Box::new(MongodbService::new()));
    }
```

Add a test mirroring `postgres_included_only_when_enabled`:

```rust
    #[test]
    fn mongodb_included_only_when_enabled() {
        let p = LaraluxPaths::new("/tmp/lara".into());
        let mut cfg = Config::default();
        assert!(!build_services(&cfg, &p).iter().any(|s| s.kind() == ServiceKind::Mongodb),
            "mongodb must be opt-in (off by default)");
        cfg.services.mongodb = true;
        assert!(build_services(&cfg, &p).iter().any(|s| s.kind() == ServiceKind::Mongodb));
    }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p laralux-core`
Expected: PASS (new `mongodb_tool_info_and_versions`, `mongodb_included_only_when_enabled`, plus all existing). Confirm the `config.rs` default/roundtrip tests still pass (the new `mongodb` field defaults false).

- [ ] **Step 6: Commit**

```bash
git add core/src/tools.rs core/src/config.rs core/src/service/registry.rs
git commit -m "feat(core): wire ManagedTool::Mongodb + opt-in services.mongodb flag"
```

---

### Task 4: Desktop command arm (`src-tauri/src/commands.rs`)

**Files:**
- Modify: `src-tauri/src/commands.rs` (`set_service_enabled` match ≈ line 134-142)

**Interfaces:**
- Consumes: `ServiceKind::Mongodb`, `ServicesConfig.mongodb`.
- Produces: nothing new (completes the exhaustive match so the desktop crate compiles).

- [ ] **Step 1: Add the `Mongodb` arm**

In `set_service_enabled`, add after the `ServiceKind::Postgres =>` arm:

```rust
        ServiceKind::Mongodb => config.services.mongodb = enabled,
```

- [ ] **Step 2: Build the desktop crate**

Run: `cargo build -p laralux-desktop`
Expected: Finished (the previously non-exhaustive `match kind` now compiles).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "feat(desktop): set_service_enabled handles Mongodb"
```

---

### Task 5: Frontend wiring (types, state, constants, dashboard)

**Files:**
- Modify: `src/ipc/types.ts` (`ServicesFlags` ≈ line 20; `ServiceStatus.kind` union ≈ line 31)
- Modify: `src/state.ts` (`services` ≈ line 128; `serviceFlags` ≈ line 129)
- Modify: `src/ui/constants.ts` (`DISP` ≈ line 9; `SVC_ORDER` ≈ line 36; `FLAG_KEY` ≈ line 40)
- Modify: `src/ui/views/dashboard.ts` (`SVC_ICON` ≈ line 9; `PORTS` ≈ line 10; `LOG_FILE` ≈ line 11; DB client card copy ≈ line 114)

**Interfaces:**
- Consumes: backend now emits/accepts the `"Mongodb"` ServiceKind and a `mongodb` flag.
- Produces: a MongoDB toggle row in Settings (auto, since the view iterates `SVC_ORDER`) and a MongoDB dashboard card when enabled (auto, via `enabledKinds()`).

> The Settings→Services view and `enabledKinds()` already iterate `SVC_ORDER`/`FLAG_KEY`/`DISP`, so adding `"Mongodb"` to those constants surfaces the toggle and card with no view-code change.

- [ ] **Step 1: Extend `src/ipc/types.ts`**

In the `ServicesFlags` interface (line 20), add `mongodb`:

```ts
  nginx: boolean; php: boolean; mariadb: boolean; redis: boolean; mailpit: boolean; postgres: boolean; mongodb: boolean;
```

In the `ServiceStatus.kind` union (line 31), add `"Mongodb"`:

```ts
  kind: "Nginx" | "PhpFpm" | "Mariadb" | "Postgres" | "Mongodb" | "Redis" | "Mailpit" | "Coredns";
```

- [ ] **Step 2: Extend `src/state.ts`**

`services` (line 128) — add `Mongodb: "Stopped"` after `Postgres`:

```ts
  services: { Nginx: "Stopped", PhpFpm: "Stopped", Mariadb: "Stopped", Postgres: "Stopped", Mongodb: "Stopped", Redis: "Stopped", Mailpit: "Stopped" },
```

`serviceFlags` (line 129) — add `mongodb: false`:

```ts
  serviceFlags: { nginx: true, php: true, mariadb: true, redis: true, mailpit: true, postgres: false, mongodb: false },
```

- [ ] **Step 3: Extend `src/ui/constants.ts`**

`DISP` (line 9) — add `Mongodb: "MongoDB"`:

```ts
export const DISP: Record<string, string> = {
  Nginx: "Nginx", PhpFpm: "PHP-FPM", Mariadb: "MariaDB", Postgres: "PostgreSQL", Mongodb: "MongoDB", Redis: "Redis", Mailpit: "Mailpit",
};
```

`SVC_ORDER` (line 36) — insert `"Mongodb"` after `"Postgres"`:

```ts
export const SVC_ORDER = ["Nginx", "PhpFpm", "Mariadb", "Postgres", "Mongodb", "Redis", "Mailpit"];
```

`FLAG_KEY` (line 40) — add `Mongodb: "mongodb"`:

```ts
export const FLAG_KEY: Record<string, string> = {
  Nginx: "nginx", PhpFpm: "php", Mariadb: "mariadb", Postgres: "postgres", Mongodb: "mongodb", Redis: "redis", Mailpit: "mailpit",
};
```

- [ ] **Step 4: Extend `src/ui/views/dashboard.ts`**

`SVC_ICON` (line 9) — add `Mongodb: I.svcMaria` (reuses the generic DB icon, as Postgres does):

```ts
const SVC_ICON: Record<string, string> = { Nginx: I.svcNginx, PhpFpm: I.svcPhp, Mariadb: I.svcMaria, Postgres: I.svcMaria, Mongodb: I.svcMaria, Redis: I.svcRedis, Mailpit: I.svcMail };
```

`PORTS` (line 10) — add `Mongodb: ["27017"]`:

```ts
const PORTS: Record<string, string[]> = { Nginx: ["80", "443"], PhpFpm: ["socket"], Mariadb: ["3306"], Postgres: ["5432"], Mongodb: ["27017"], Redis: ["6379"], Mailpit: ["8025", "1025"] };
```

`LOG_FILE` (line 11) — add `Mongodb: "mongodb.log"`:

```ts
const LOG_FILE: Record<string, string> = { Nginx: "nginx-error.log", PhpFpm: "php-fpm.log", Mariadb: "mariadb.log", Postgres: "postgres.log", Mongodb: "mongodb.log", Redis: "redis.log", Mailpit: "mailpit.log" };
```

DB client card copy (line 114) — update to include MongoDB:

```ts
    '<div class="site-desc">DbGate — manage MariaDB, PostgreSQL, MongoDB &amp; Redis</div></div>' +
```

- [ ] **Step 5: Build the frontend (strict tsc)**

Run: `npm run build`
Expected: `✓ built` with no TypeScript errors (strict mode, `noUnusedLocals`/`noUnusedParameters`).

- [ ] **Step 6: Commit**

```bash
git add src/ipc/types.ts src/state.ts src/ui/constants.ts src/ui/views/dashboard.ts
git commit -m "feat(ui): MongoDB service card + Settings toggle + DB client copy"
```

---

## Self-Review

- **Spec coverage:** §2 installer → Task 1; §3 service → Task 2; §4 tool/config/registry → Task 3; §5 desktop command → Task 4, frontend → Task 5; §8 tests are embedded in Tasks 1-3 and 5. The reused foundation (reconcile, set_service_enabled flow, Settings view, enabledKinds) needs no new task — confirmed present in the read code.
- **Placeholder scan:** none — every step has full code.
- **Type consistency:** `ServiceKind::Mongodb`, `ManagedTool::Mongodb` (key `"mongodb"`), `ServicesConfig.mongodb`, `FLAG_KEY.Mongodb = "mongodb"`, `SVC_ORDER` includes `"Mongodb"`, `ServiceStatus.kind` union includes `"Mongodb"` — all aligned. `MongodbService::new()` matches the `Box::new(MongodbService::new())` call in registry. The `open-tool` data-tool value (`FLAG_KEY["Mongodb"] = "mongodb"`) matches `ToolInfo.key = "mongodb"` so Manage resolves the tool.
- **Compile-order note:** Task 2 intentionally leaves the desktop crate non-exhaustive; Task 4 closes it. Each task's stated verification command (core test / desktop build / npm build) is green at that task's boundary.
