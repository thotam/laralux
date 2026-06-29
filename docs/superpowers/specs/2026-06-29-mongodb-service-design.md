# Laralux — MongoDB (opt-in managed service)

**Date:** 2026-06-29
**Status:** Design (approved for spec).
**Goal:** Add **MongoDB** as a no-apt, multi-version managed service that is **opt-in** (off by
default), installed as an official static tarball into `~/laralux/` like the other tools,
orchestrated (Start/Stop, dbpath, health), reachable from the bundled DbGate client, with the
`mongosh` shell bundled as a CLI.

Second of the three Phase-3 database sub-projects (PostgreSQL shipped first and built the
service enable/disable foundation; Memcached follows as its own spec). MongoDB reuses that
foundation wholesale — this sub-project adds one installer, one service, and the toggle/dashboard
wiring, with **no new UI mechanism**.

---

## 1. Context & current state

- Every managed tool follows one no-apt pattern: a `core/src/<tool>_static.rs` downloads a static
  binary/tarball into `bin/<key>/<version>/` (with top-level symlinks to the executables), and a
  `core/src/service/<name>.rs` implements the `Service` trait (`kind`, `name`, `write_config`,
  `command`, `health_check`, plus `needs_init`/`init` for services that need datadir setup).
  `core/src/service/registry.rs::build_services(config, paths)` builds the enabled set from
  `config.services.*` flags; the orchestrator owns process lifecycle.
- The PostgreSQL sub-project added the **service enable/disable foundation**:
  `ServicesConfig` carries per-service opt-in flags (`#[serde(default)]` so old configs load),
  `Orchestrator::reconcile(new_services)` stops removed kinds and swaps the registered set, the
  `set_service_enabled(kind, enabled)` Tauri command saves the flag + reconciles + returns the
  snapshot, and a **Settings → Services** section toggles each service. The dashboard renders the
  enabled set (`SVC_ORDER` filtered by `state.serviceFlags[FLAG_KEY[k]]`), so a newly-enabled
  service appears as a card automatically and a disabled one disappears. **MongoDB plugs into all of
  this; it introduces no new toggle/reconcile mechanism.**
- Services self-log via their own config (nginx `error_log`, mariadb `log-error`, postgres logging
  collector): `SpawnSpec` has no stdout/stderr redirection, so MongoDB must use mongod's own
  `--logpath`.
- `ServiceKind` derives `Serialize`/`Deserialize` (usable as a Tauri command argument).
- DbGate (the bundled DB client) supports **MongoDB** connections in its free build.
- **MongoDB portable binaries (verified, HTTP 200):**
  - Server tarball (official, plain gzip — no jar/zip unwrap):
    `https://fastdl.mongodb.org/linux/mongodb-linux-<arch>-ubuntu2204-<ver>.tgz`
    where `<arch>` is `x86_64` or `aarch64`. Extracts to a single wrapper dir
    `mongodb-linux-<arch>-ubuntu2204-<ver>/` containing `bin/mongod`, `bin/mongos`, `LICENSE-*`.
    Verified present: `8.0.4`, `7.0.15`. (The `ubuntu2204` build runs on modern glibc distros
    generally; chosen for broad compatibility.)
  - `mongosh` shell (official, plain gzip, separate cadence — Apache-2.0):
    `https://downloads.mongodb.com/compass/mongosh-<ver>-linux-<arch2>.tgz`
    where `<arch2>` is `x64` or `arm64`. Extracts to `mongosh-<ver>-linux-<arch2>/bin/mongosh`
    (+ `mongosh_crypt_v1.so`). Verified present: `2.3.8`.
- **Licensing:** the MongoDB server is **SSPL** (not OSI-approved). Bundling official binaries for
  local development use is acceptable for this tool (Laralux downloads them onto the developer's own
  machine; it does not redistribute or offer MongoDB as a hosted service). `mongosh` is Apache-2.0.
  A short note in the install path / docs suffices; no license gate in code.

## 2. Install — `core/src/mongodb_static.rs`

Mirrors `postgres_static.rs` (multi-version, idempotent, top-level symlinks) but simpler: both
artifacts are plain gzip tarballs, so there is **no jar/zip unwrap and no `zip` crate** — just
`tar -xzf --strip-components=1`.

- `pub const MONGODB_VERSION: &str = "8.0.4";`
- `pub const KNOWN_MONGODB_VERSIONS: [&str; 2] = ["8.0.4", "7.0.15"];` (both verified on fastdl).
- `pub const MONGOSH_VERSION: &str = "2.3.8";` — the bundled shell, pinned (independent of the
  server version, like MongoDB's own separate distribution).
- `pub fn mongodb_arch() -> Option<&'static str>`: `x86_64` → `x86_64`, `aarch64` → `aarch64`, else `None`.
- `pub fn mongosh_arch() -> Option<&'static str>`: `x86_64` → `x64`, `aarch64` → `arm64`, else `None`.
- `pub fn mongodb_url(version, arch) -> String`:
  `https://fastdl.mongodb.org/linux/mongodb-linux-{arch}-ubuntu2204-{version}.tgz`.
- `pub fn mongosh_url(version, arch2) -> String`:
  `https://downloads.mongodb.com/compass/mongosh-{version}-linux-{arch2}.tgz`.
- `pub fn install_mongodb(paths, downloader, runner, sink)` and
  `install_mongodb_version(paths, version, downloader, runner, sink) -> Result<String, MongodbError>`:
  - Idempotent: no-op (just `set_current`) if `bin/mongodb/<version>/bin/mongod` already exists.
  - Download the server tarball to `tmp/mongodb.tgz` (drives the progress ring).
  - `tar -xzf tmp/mongodb.tgz --strip-components=1 -C bin/mongodb/<version>/` via `CommandRunner`
    (flattens the wrapper dir → `bin/mongod`, `bin/mongos` land directly under the version dir).
  - Download `mongosh` (`MONGOSH_VERSION`) to `tmp/mongosh.tgz`, then
    `tar -xzf tmp/mongosh.tgz --strip-components=1 -C bin/mongodb/<version>/` — its `bin/mongosh`
    merges into the same `bin/`. (mongosh download/extract failure is non-fatal: the server still
    works; log and continue. The server tarball failing IS fatal.)
  - Verify `bin/mongodb/<version>/bin/mongod` exists after extract; error if missing.
  - Create top-level symlinks in `bin/mongodb/<version>/` for `mongod`, `mongos`, and (if present)
    `mongosh` → `bin/<exe>`, so `bin/mongodb/current/<exe>` resolves (matching `cli_paths` and the
    orchestrator `resolve_or_name` conventions).
  - Remove the downloaded tarballs; `crate::layout::set_current(paths, "mongodb", version)`.
- `#[derive(thiserror::Error)] pub enum MongodbError { Arch(String), Download(String), Extract(String), Io(#[from] std::io::Error) }`.
- No new crate dependency (plain `tar -xzf`).
- Export the version consts + `install_mongodb*` + `MongodbError` from `lib.rs`.

## 3. Service — `core/src/service/mongodb.rs`

Stateless-init (mongod creates its own dbpath contents on first start — no `initdb` analogue), so it
is **simpler than `PostgresService`**: no `needs_init`/`init` override (the `Service` trait defaults
apply), just ensure the dbpath dir exists in `write_config`.

- Port `27017`, bind `127.0.0.1`; unix socket prefix = `tmp/`.
- `kind()` = `ServiceKind::Mongodb`, `name()` = `"mongodb"`.
- `write_config`: ensure `data/mongodb`, `log/`, and `tmp/` exist. mongod requires the dbpath
  directory to pre-exist (it does not create it); it populates the WiredTiger files itself on first
  start, so there is no separate init step.
- `command(paths)`: `mongod --dbpath <data/mongodb> --port 27017 --bind_ip 127.0.0.1
  --unixSocketPrefix <tmp> --logpath <log/mongodb.log> --logappend` — mongod's own `--logpath`
  writes `log/mongodb.log` (SpawnSpec can't redirect stdout/stderr); **no `--fork`** so mongod stays
  in the foreground for the orchestrator to supervise.
- `health_check`: `probe_tcp(27017)`.
- `pre_start`: clear a stale `data/mongodb/mongod.lock` and the `tmp/mongodb-27017.sock` from a
  previous run via `crate::service::cleanup_stale_endpoint` (same pattern as Postgres's
  `postmaster.pid` + socket).

## 4. Tool & stack integration

- `core/src/tools.rs`: add `ManagedTool::Mongodb` → `ToolInfo { key: "mongodb", display: "MongoDB",
  cli_binaries: &["mongod", "mongosh"], service_kind: Some(ServiceKind::Mongodb) }`. Add to
  `ManagedTool::ALL`, wire `available_versions` (`known_catalog` over `KNOWN_MONGODB_VERSIONS`) and
  `install_version` dispatch (`install_mongodb_version`), same as Postgres.
- `core/src/service/mod.rs`: add `ServiceKind::Mongodb` and `pub mod mongodb;`.
- `core/src/service/registry.rs`: include `MongodbService` when `config.services.mongodb` is true.
- `core/src/config.rs`: add `mongodb: bool` to `ServicesConfig`, **default `false`** (opt-in) with
  `#[serde(default)]`. Older config files without the field deserialize to `false`.
- Start order: MongoDB is an independent DB, same tier as mariadb/redis/postgres (before
  php-fpm/nginx); the orchestrator's topological order handles it (no inter-DB deps).
- Setup/version modal: MongoDB appears as an installable ManagedTool (install + pick version) like
  the others — no Setup-wizard change needed beyond the new ManagedTool entry.

## 5. Service enable/disable + dashboard (reused, not rebuilt)

No new mechanism. MongoDB rides the PostgreSQL-era foundation:

- **Desktop** (`src-tauri/src/commands.rs`): `set_service_enabled` gains one arm —
  `ServiceKind::Mongodb => config.services.mongodb = enabled`. The save → `reconcile` →
  snapshot flow is unchanged.
- **Frontend**: the **Settings → Services** section already iterates `SVC_ORDER`; adding
  `"Mongodb"` to `SVC_ORDER`, `DISP`, and `FLAG_KEY` makes the toggle row appear with no new code.
  `state.serviceFlags` gains `mongodb: false`; `state.services` gains `Mongodb: "Stopped"`.
  `ServicesFlags` (types.ts) gains `mongodb: boolean`; `ServiceStatus.kind` union gains `"Mongodb"`.
  The dashboard's `SVC_ICON`/`PORTS`/`LOG_FILE` gain `Mongodb` entries (icon reuses `I.svcMaria` as
  Postgres does; port `27017`; `mongodb.log`). DB client card copy →
  "DbGate — manage MariaDB, PostgreSQL, MongoDB & Redis".
- The dashboard derives cards from `enabledKinds()` (SVC_ORDER ∩ enabled flags), so an enabled
  MongoDB shows a card and a disabled one disappears — no fixed service-count assumption.

## 6. Data flow
1. Settings → Services → enable MongoDB → `setServiceEnabled("Mongodb", true)` → config saved →
   `reconcile` registers `MongodbService` → snapshot includes Mongodb (Stopped) → card appears.
2. Setup → install MongoDB (pick version) → `install_mongodb_version` (download ring: server tgz +
   mongosh) → binaries in `bin/mongodb/<ver>/`.
3. Start All (or the card's Start) → orchestrator `start(Mongodb)` → mongod launches on
   127.0.0.1:27017, logging to `log/mongodb.log` (creates WiredTiger files in `data/mongodb` on
   first run).
4. DbGate → new MongoDB connection `mongodb://127.0.0.1:27017` (no auth) → lists databases.
5. Disable MongoDB → `setServiceEnabled("Mongodb", false)` → reconcile stops it + drops the card.

## 7. Error handling
- Server-tarball install failures (`Arch`, download, extract) → toast + cleared busy, like other
  installers. The optional `mongosh` download failing is **non-fatal** (logged; server usable).
- mongod start failures surface through the orchestrator's normal `Crashed`/error path (log link to
  `log/mongodb.log`); no infinite restart loop.
- `reconcile` disabling a running MongoDB stops it cleanly (no orphan); best-effort, mirroring
  `stop_all`.

## 8. Testing (TDD where it applies)
- `mongodb_static.rs`: `mongodb_arch()`/`mongosh_arch()` mappings; `mongodb_url()` and
  `mongosh_url()` exact for x86_64/x64 and aarch64/arm64; `install_*` idempotency guard (skips when
  `bin/mongodb/<ver>/bin/mongod` exists); `KNOWN_MONGODB_VERSIONS` contains `MONGODB_VERSION`.
  (Real download/extract not unit-tested, like other installers.)
- `service/mongodb.rs`: `kind`/`name`; `command` contains the dbpath, port `27017`,
  `--bind_ip 127.0.0.1`, `--unixSocketPrefix`, and `--logpath …/mongodb.log`.
- `tools.rs`: `ManagedTool::Mongodb` key/`service_kind`/version count.
- `registry.rs`: `build_services` includes Mongodb iff `config.services.mongodb`.
- `config.rs`: `mongodb` defaults false; old config without the field loads.
- `cargo test -p laralux-core` green; `cargo build -p laralux-desktop` green; `npm run build` green.
- Manual smoke (needs a display): Settings → enable MongoDB → Setup install → Start → DbGate
  connects to `mongodb://127.0.0.1:27017` and lists databases; disable → card disappears and the
  process stops.

## 9. Out of scope / backlog
- Memcached (separate Phase-3 spec).
- MongoDB authentication / replica sets / sharding (single standalone mongod, no auth, like the
  password-less MariaDB/Postgres defaults).
- A dedicated MongoDB dashboard icon (reuses the generic DB icon, like Postgres — backlog).
- Per-site MongoDB provisioning / auto-create database on site scaffold.
- Expanding `KNOWN_MONGODB_VERSIONS` beyond the two verified entries / a 6.0 LTS line.
