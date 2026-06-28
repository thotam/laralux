# Laralux — PostgreSQL (opt-in managed service)

**Date:** 2026-06-28
**Status:** Design (approved for spec).
**Goal:** Add **PostgreSQL** as a no-apt, multi-version managed service that is **opt-in** (off by
default), installed as a static/portable binary into `~/laralux/` like the other tools, orchestrated
(Start/Stop, datadir init, health), and reachable from the bundled DbGate client.

First of the three Phase-3 database sub-projects (MongoDB and Memcached follow as their own
specs). This sub-project also introduces a **service enable/disable toggle** — the foundation that
makes PostgreSQL (and later Mongo/Memcached) opt-in.

---

## 1. Context & current state

- Every managed tool follows one no-apt pattern: a `core/src/<tool>_static.rs` downloads a static
  binary/tarball into `bin/<key>/<version>/` (with top-level symlinks to the executables), and a
  `core/src/service/<name>.rs` implements the `Service` trait (`kind`, `name`, `write_config`,
  `command`, `health_check`, plus `needs_init`/`init` for stateful services like MariaDB).
  `core/src/service/registry.rs::build_services(config, paths)` builds the enabled set from
  `config.services.*` flags; the orchestrator owns process lifecycle.
- Services self-log via their own config (nginx `error_log`, mariadb `log-error`, redis `logfile`):
  `SpawnSpec` has no stdout/stderr redirection, so PostgreSQL must use its own logging collector.
- `config.services` (`ServicesConfig`) has five always-on flags (nginx/php/mariadb/redis/mailpit, all
  default `true`); there is **no UI to toggle services** today. The dashboard shows a fixed set of
  service cards.
- `ServiceKind` derives `Serialize`/`Deserialize` (usable as a Tauri command argument).
- The Setup component list installs ManagedTools (version + symlink modal). DbGate (the bundled DB
  client) already supports PostgreSQL connections.
- **PostgreSQL portable binaries:** the **zonky embedded-postgres** artifacts on Maven Central are a
  clean, version-parameterized, fully portable source (verified):
  `https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-linux-<arch>/<ver>/embedded-postgres-binaries-linux-<arch>-<ver>.jar`
  where `<arch>` is `amd64` or `arm64v8`. The `.jar` is a zip containing exactly one `*.txz` entry
  (`postgres-linux-x86_64.txz` for amd64) which untars to a standard PG layout (`bin/`, `lib/`,
  `share/` — `bin/postgres`, `bin/initdb`, `bin/psql`, `bin/pg_ctl`, …). Versions 17.2.0 / 16.6.0 /
  15.8.0 / 14.13.0 are present.

## 2. Install — `core/src/postgres_static.rs`

Mirrors `mariadb_static.rs` (multi-version, idempotent, top-level symlinks), with an extra unwrap of
the zonky jar.

- `pub const POSTGRES_VERSION: &str = "16.6.0";`
- `pub const KNOWN_POSTGRES_VERSIONS: [&str; 4] = ["17.2.0", "16.6.0", "15.8.0", "14.13.0"];`
- `pub fn postgres_arch() -> Option<&'static str>`: `x86_64` → `amd64`, `aarch64` → `arm64v8`, else `None`.
- `pub fn postgres_url(version, arch) -> String`: the Maven jar URL above.
- `pub fn install_postgres(paths, downloader, runner, sink)` and
  `install_postgres_version(paths, version, downloader, runner, sink) -> Result<(), PostgresError>`:
  - Idempotent: no-op if `bin/postgres/<version>/bin/postgres` already exists.
  - Download the jar to `tmp/postgres.jar` (drives the progress ring).
  - **Extract the single `*.txz` entry from the jar using the `zip` crate** (pure Rust — no `unzip`
    system dependency) to `tmp/postgres.txz`. (The entry name's arch token varies, so select by the
    `.txz` suffix, not a hardcoded name.)
  - `tar -xJf tmp/postgres.txz -C bin/postgres/<version>/` via `CommandRunner` (xz — base on Ubuntu),
    yielding `bin/`, `lib/`, `share/` under the version dir.
  - Create top-level symlinks in `bin/postgres/<version>/` for `postgres`, `initdb`, `pg_ctl`, `psql`,
    `pg_dump`, `pg_restore`, `createdb`, `dropdb` → `bin/<exe>`, so `bin/postgres/current/<exe>`
    resolves (matching the `cli_paths` and orchestrator `resolve_or_name` conventions).
  - Remove the downloaded jar/txz.
- `#[derive(thiserror::Error)] pub enum PostgresError { Arch(String), Download(String), Extract(String), Io(#[from] std::io::Error) }`.
- Add the `zip` crate to `laralux-core`'s dependencies (laralux-core stays Tauri-free; `zip` is fine).
- Export the version lists + `install_postgres*` from `lib.rs`.

## 3. Service — `core/src/service/postgres.rs`

Stateful, modeled on `MariadbService`.

- Port `5432`, bind `127.0.0.1`; unix socket directory = `tmp/`.
- `kind()` = `ServiceKind::Postgres`, `name()` = `"postgres"`.
- `needs_init(paths)`: true when `data/postgres/PG_VERSION` is absent (empty datadir).
- `init(paths)`: `initdb -D data/postgres -U postgres --auth=trust --encoding=UTF8` (run via the
  resolved `bin/postgres/current/initdb`). Superuser `postgres`, **no password**, local `trust` auth —
  the PostgreSQL analogue of MariaDB's password-less root.
- `write_config`: ensure `data/postgres` and `log/` exist. PostgreSQL keeps its config inside the
  datadir (generated by `initdb`); runtime overrides are passed on the command line, so no separate
  config file is written.
- `command(paths)`: `postgres -D data/postgres -p 5432 -k <tmp> -c listen_addresses=127.0.0.1
  -c logging_collector=on -c log_directory=<log> -c log_filename=postgres.log` — PG's logging
  collector writes `log/postgres.log` (SpawnSpec can't redirect stdout/stderr).
- `health_check`: `probe_tcp(5432)`.

## 4. Tool & stack integration

- `core/src/tools.rs`: add `ManagedTool::Postgres` → `ToolInfo { key: "postgres", display:
  "PostgreSQL", cli_binaries: &["psql", "pg_dump", "pg_restore", "createdb", "dropdb"],
  service_kind: Some(ServiceKind::Postgres) }`. Add to `ManagedTool::ALL`. Wire version
  install/switch through the existing `set_tool_version`/`install_tool_version` paths and Setup modal
  (same as Node/MariaDB).
- `core/src/service/mod.rs`: add `ServiceKind::Postgres`.
- `core/src/service/registry.rs`: include `PostgresService` when `config.services.postgres` is true.
- `core/src/config.rs`: add `postgres: bool` to `ServicesConfig`, **default `false`** (opt-in). Older
  config files without the field deserialize to `false`.
- Start order: PostgreSQL is an independent DB, same tier as mariadb/redis (before php-fpm/nginx).
- Setup: PostgreSQL appears as an installable ManagedTool (install + pick version) like the others.

## 5. Service enable/disable toggle (new foundation)

Opt-in needs a way to turn a service on/off at runtime without restarting the app or orphaning
processes.

- **Orchestrator** (`core/src/orchestrator.rs`): add
  `pub fn reconcile(&mut self, new_services: Vec<Box<dyn Service>>)`. It replaces `self.services` with
  `new_services`; for every `ServiceKind` present before but absent now, it `stop()`s the service
  (terminating the child, dropping its handle) and removes its `states` entry. Running handles for
  kinds that survive are preserved (handles are keyed by kind). This reuses `build_services` as the
  source of truth — the controller calls `reconcile(build_services(&config, &paths))` after a flag
  change.
- **Desktop** (`src-tauri/src/commands.rs`): `#[tauri::command] pub fn set_service_enabled(state,
  kind: ServiceKind, enabled: bool) -> Result<Vec<ServiceStatus>, String>`: load config, set
  `config.services.<kind>` (map `ServiceKind` → the flag), save, then
  `orch.reconcile(build_services(&config, &paths))`, and return the new snapshot. Register in
  `main.rs`.
- **Frontend**: a **"Services"** section in Settings listing each toggleable service with an on/off
  switch (the five core default on, PostgreSQL off). Toggling calls `setServiceEnabled(kind, enabled)`
  and refreshes the snapshot. When PostgreSQL is enabled **and installed**, it starts with Start All
  and shows a service card; when disabled it is hidden and never started. (If enabled but not yet
  installed, its card/Setup entry offers install via the existing tool-install flow.)
- The dashboard's service cards derive from the orchestrator snapshot (registered services), so an
  enabled PostgreSQL appears automatically and a disabled one disappears — no fixed 5-service
  assumption remains in the rendered list.

## 6. Data flow
1. Settings → Services → enable PostgreSQL → `set_service_enabled(Postgres, true)` → config saved →
   `reconcile` registers `PostgresService` → snapshot now includes Postgres (Stopped) → card appears.
2. Setup → install PostgreSQL (pick version) → `install_postgres_version` (download ring) → binaries in
   `bin/postgres/<ver>/`.
3. Start All (or the card's Start) → orchestrator `start(Postgres)` → first run `initdb` (datadir) →
   `postgres` launches on 127.0.0.1:5432, logging to `log/postgres.log`.
4. DbGate → new PostgreSQL connection `127.0.0.1:5432`, user `postgres`, empty password.
5. Disable PostgreSQL → `set_service_enabled(Postgres, false)` → reconcile stops it + drops the card.

## 7. Error handling
- Install failures (`Arch`, download, jar/txz extract) → toast + cleared busy, like other installers.
- `initdb`/start failures surface through the orchestrator's normal `Crashed`/error path (log link to
  `log/postgres.log`); no infinite restart loop.
- `reconcile` disabling a running service stops it cleanly (no orphan); best-effort, mirroring
  `stop_all`.

## 8. Testing (TDD where it applies)
- `postgres_static.rs`: `postgres_arch()` mapping; `postgres_url()` exact for amd64 and arm64v8;
  `install_*` idempotency guard (skips when `bin/postgres/<ver>/bin/postgres` exists). (Real
  download/jar-unwrap/extract not unit-tested, like other installers.)
- `service/postgres.rs`: `kind`/`name`; `needs_init` true on empty datadir and false once
  `PG_VERSION` exists; `command` contains the datadir, port, socket dir, `listen_addresses`, and the
  logging-collector flags; `init` builds the expected `initdb` argument list.
- `orchestrator.rs`: `reconcile` adds a newly-enabled kind, and removing a kind stops it (its handle
  is gone and state cleared) while a surviving running kind keeps its handle.
- `registry.rs`: `build_services` includes Postgres iff `config.services.postgres`.
- `cargo test -p laralux-core` green; `cargo build -p laralux-desktop` green; `npm run build` green.
- Manual smoke (needs a display): Settings → enable PostgreSQL → Setup install → Start → DbGate
  connects to 127.0.0.1:5432 (user `postgres`) and lists databases; disable → card disappears and the
  process stops.

## 9. Out of scope / backlog
- MongoDB and Memcached (separate Phase-3 specs).
- PostgreSQL extensions, multiple clusters, or non-default locales/collations.
- Per-site PostgreSQL provisioning / auto-create database on site scaffold.
- Migrating the existing five services to be individually toggleable beyond what the new Services
  section already exposes (the toggle works for all registered services; defaults keep the five on).
