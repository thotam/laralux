# Laralux — Per-site Procfile process runner

**Date:** 2026-06-29
**Status:** Design (approved for spec).
**Goal:** Let each site declare background processes in a standard **Procfile**, which Laralux
supervises (Start/Stop, status, logs) alongside the infrastructure stack — a Foreman-style runner
for queue workers, schedulers, asset watchers, etc. Each site has an **autostart** flag so its
processes start with the global "Start All".

---

## 1. Context & current state

- **Process primitives already exist and are reusable.** `core/src/process.rs` defines
  `trait Process { is_alive; stop; reload; pid }` and `trait ProcessSpawner { spawn(&SpawnSpec) }`,
  with `RealSpawner` (real `std::process::Child`, SIGTERM→SIGKILL stop) and `FakeSpawner` (tests).
  `SpawnSpec { program, args, env, cwd }` (in `core/src/service/mod.rs`) fully describes a command —
  it has **no stdout/stderr redirection** (services self-log via their own config).
- **The `Orchestrator` is NOT reusable for this.** It keys handles by a fixed `ServiceKind` enum
  (`HashMap<ServiceKind, Box<dyn Process>>`, one process per kind). Per-site Procfile processes are
  dynamic (arbitrary names, many per site, many sites), so they need a **separate supervisor** that
  reuses the `ProcessSpawner`/`Process`/`SpawnSpec` primitives but is keyed by `(site, proc)`.
- **Sites.** `core/src/sites.rs::Site { name: String, root: PathBuf, hostname: String }`. Sites come
  from scanning `~/laralux/www/` plus the site registry (proxies/linked folders). A site's Procfile
  lives at `<site.root>/Procfile`. Proxy sites have no local folder → no Procfile.
- **Config** (`core/src/config.rs`) already carries simple per-feature collections like
  `symlinks: BTreeSet<String>` with `#[serde(default)]`; the autostart flag follows that pattern.
- **Frontend.** The Sites view (`src/ui/views/sites.ts`) renders site rows with a kebab **row-menu**
  (Copy URL, Domains, Edit proxy, Delete) and uses modals for Domains/Proxy (mirrors the pattern to
  follow). The desktop monitor thread (`src-tauri/src/main.rs`, ~1s) already polls the orchestrator
  and emits `services-changed`; an analogous `site-procs-changed` event fits the same loop.
- **Paths:** `paths.log()` is `~/laralux/log`; managed tool bins resolve via
  `crate::layout::managed_bin_dirs(paths)` (the `bin/*/current` dirs); `/usr/local/bin` holds the
  optional tool symlinks.

## 2. Procfile format — `core/src/procfile.rs`

Standard Foreman Procfile (pure parser, no I/O in the parse fn → trivially testable):

- `pub struct ProcEntry { pub name: String, pub command: String }`.
- `pub fn parse_procfile(text: &str) -> Vec<ProcEntry>`:
  - One entry per line of the form `name: command`.
  - `name` matches `[A-Za-z0-9_-]+`; everything after the first `:` (trimmed) is `command`.
  - Blank lines and lines whose first non-space char is `#` are ignored.
  - Lines that don't match `name:` (no colon, or invalid name) are skipped (lenient — a malformed
    line never aborts the whole file). Duplicate names: last one wins is NOT required; keep first and
    skip later duplicates (deterministic).
- `pub fn read_procfile(site_root: &Path) -> Option<Vec<ProcEntry>>`: reads `<site_root>/Procfile`;
  returns `None` if the file is absent, `Some(entries)` otherwise (possibly empty).

## 3. Supervisor — `core/src/site_procs.rs`

A standalone supervisor reusing the process primitives. Keyed by `(site_name, proc_name)`.

- `pub struct SiteProcs { paths: LaraluxPaths, spawner: Box<dyn ProcessSpawner>, handles:
  HashMap<(String,String), Box<dyn Process>>, states: HashMap<(String,String), ProcState> }`.
- `ProcState` reuses the existing `crate::service::ServiceState` (Stopped/Starting/Running/
  Stopping/Crashed) for one status vocabulary across the UI. (No new enum.)
- `pub struct ProcStatus { pub site: String, pub name: String, pub command: String, pub state:
  ServiceState, pub pid: Option<u32> }` — what the snapshot/commands return (Serialize).
- **Spawn shape (logging + correct signal target):** each process is launched as
  `SpawnSpec::new("sh").arg("-c").arg(format!("exec {command} >> {log} 2>&1"))` with `cwd = site
  root` and `env PATH = <managed_bin_dirs joined>:/usr/local/bin:<inherited PATH>`. `exec` makes the
  tracked PID the real process (so `stop()`'s SIGTERM hits the worker, not a wrapper shell); the
  shell `>>` redirect captures stdout+stderr **without** changing `SpawnSpec` (which can't redirect).
  Log file: `paths.log().join(format!("proc-{site}-{name}.log"))`.
- Methods:
  - `start(&mut self, site, root, name, command)` — idempotent (no-op if a live handle exists);
    ensures `log/` exists; spawns; records handle + `Running` (or `Crashed` on spawn error).
  - `stop(&mut self, site, name)` — stop + drop handle, state `Stopped`.
  - `start_site(&mut self, site, root)` — read Procfile, `start` each entry.
  - `stop_site(&mut self, site)` — stop every handle whose key's site matches.
  - `refresh(&mut self)` — for each handle, `is_alive()`; a handle that died unexpectedly →
    `Crashed` (handle dropped). Mirrors `Orchestrator::refresh`.
  - `snapshot(&self, entries_by_site) -> Vec<ProcStatus>` — merges declared Procfile entries with
    live states (declared-but-not-running shows `Stopped`). The desktop layer passes the parsed
    entries (it owns site enumeration).
  - `stop_all(&mut self)` — stop every handle (used by global Stop All / app exit).
  - No auto-restart: a `Crashed` proc stays crashed until the user restarts it (matches services).

## 4. Autostart flag — `core/src/config.rs`

- Add `#[serde(default)] pub proc_autostart: BTreeSet<String>` to `Config` (site names whose procs
  start with the global stack). Default empty; old configs without the field load (serde default).
- Helpers on `Config`: `proc_autostart_enabled(site) -> bool`, and the desktop toggles membership.

## 5. Desktop integration — `src-tauri/src/commands.rs` + `main.rs`

- `AppState` gains `site_procs: Mutex<SiteProcs>` (built with `RealSpawner`).
- Commands (all return the fresh `Vec<ProcStatus>` for the site unless noted):
  - `site_procs(name, root) -> Vec<ProcStatus>` — parse `<root>/Procfile`, merge with live states.
    Returns an empty vec when there is no Procfile (frontend shows "No Procfile").
  - `start_site_proc(name, root, proc)` / `stop_site_proc(name, proc)`.
  - `start_site_procs(name, root)` / `stop_site_procs(name)` — all procs of one site.
  - `set_site_autostart(name, enabled) -> bool` — add/remove `name` in `config.proc_autostart`,
    save, return the new state.
  - `site_proc_log_path(name, proc) -> String` — the `proc-<site>-<proc>.log` path (the existing
    log-viewing affordance / toast pattern reuses this).
- The frontend passes both `name` and `root` (it holds the `Site` object); the supervisor keys on
  `name`, runs in `root`. (Commands that only stop/lookup by key take just `name`.)
- **Monitor thread:** in the existing ~1s loop, `site_procs.lock().refresh()`, build the snapshot for
  sites that have running procs, and emit `site-procs-changed` only when it changes (same
  changed-only pattern as `services-changed`).
- **Start All / Stop All:** `run_full_start` — after the stack is up, for each site in
  `config.proc_autostart` call `start_site_procs`. The Stop-All path and app-exit (`ExitRequested`)
  also call `site_procs.stop_all()` so no worker is orphaned.

## 6. Frontend

- **Row indicator:** a site whose folder has a Procfile shows a small chip "⚙ N" (process count) on
  its row. (The site list payload gains an optional `procCount` per site, computed when sites are
  loaded; sites without a Procfile show nothing.)
- **Row-menu item "Processes":** opens a **Processes modal** for that site:
  - Header: site name + a **Autostart** toggle (`set_site_autostart`) + "Start all" / "Stop all".
  - A row per declared proc: status dot (reusing `META`/`ServiceState` styling), the proc name, the
    command (muted/monospace), a **Start/Stop** button, and **View logs** (toast with the
    `proc-<site>-<proc>.log` path, like the service crash log affordance).
  - Empty state when there is no Procfile: "No Procfile in this site's folder."
- State: `state.siteProcs` (the open modal's `Vec<ProcStatus>` + site) and `state.procModal` (which
  site is open). The `site-procs-changed` event refreshes the open modal live.
- IPC: `src/ipc/types.ts` `ProcStatus` interface + `ServiceState` reuse; `src/ipc/commands.ts`
  wrappers for the six commands; `src/ipc/events.ts` subscribes to `site-procs-changed`.

## 7. Data flow
1. Site folder has `Procfile` → site list shows "⚙ N"; row-menu → Processes opens the modal
   (`site_procs(name,root)` parses + merges states).
2. Start a proc → `start_site_proc` → `sh -c 'exec <cmd> >> log 2>&1'` in the site root → state
   `Running`, logging to `proc-<site>-<proc>.log`.
3. Monitor detects a crash within ~1s → `site-procs-changed` → modal row flips to `Crashed` + View
   logs.
4. Toggle Autostart on → site name added to `config.proc_autostart`. Next "Start All" also starts
   that site's procs; "Stop All" stops them.
5. Stop a proc / Stop all / app exit → SIGTERM→SIGKILL via the existing `Process::stop`.

## 8. Error handling
- No Procfile → no controls, empty modal state (not an error).
- Malformed Procfile line → skipped by the parser; the rest still run.
- Spawn failure (bad command) → that proc shows `Crashed`; `View logs` points at its log file (the
  shell writes the error there). Other procs are unaffected.
- A proc that exits on its own (worker finished/crashed) → `refresh` marks it `Crashed`; no
  auto-restart (consistent with the service model).
- `stop_all` on exit is best-effort (mirrors `Orchestrator::stop_all`).

## 9. Testing (TDD where it applies)
- `procfile.rs`: `parse_procfile` — `name: cmd` parsing, `#` comments, blank lines, a malformed line
  skipped while valid lines survive, a command containing `:` kept intact, duplicate-name handling.
  `read_procfile` — `None` when absent, `Some` when present.
- `site_procs.rs` (via `FakeSpawner`): `start` records a `sh -c exec … >> … 2>&1` spec with the
  right cwd and PATH; `start_site` starts every Procfile entry; `stop`/`stop_site`/`stop_all` drop
  handles and clear state; `refresh` flips a dead handle to `Crashed`; `snapshot` merges declared
  entries (Stopped) with running ones; `start` is idempotent for a live handle.
- `config.rs`: `proc_autostart` defaults empty, roundtrips, old config without the field loads.
- `cargo test -p laralux-core` green; `cargo build -p laralux-desktop` green; `npm run build` green.
- Manual smoke (needs a display): a site with `Procfile` (`worker: php artisan queue:work`) → row
  chip; Processes modal → Start → Running + log file fills; crash → Crashed within ~1s; Autostart on
  → "Start All" launches it; "Stop All" stops it.

## 10. Out of scope / backlog
- Foreman `$PORT` assignment (incrementing per process) and `name=N` concurrency.
- Auto-restart / backoff supervision (deliberately not done — matches the service model).
- Editing the Procfile from the UI (it is read-only; users edit the file in their project).
- A full streaming log viewer in-app (v1 surfaces the log file path, like service crash logs).
- `.env` loading for procs beyond inheriting PATH + the managed tool bins.
