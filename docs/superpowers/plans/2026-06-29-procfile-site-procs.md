# Per-site Procfile Process Runner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let each site declare background processes in a Foreman-style `Procfile` that Laralux supervises (Start/Stop, status, logs), with a per-site autostart flag so a site's processes start with the global "Start All".

**Architecture:** A new `SiteProcs` supervisor in `laralux-core`, separate from the `ServiceKind`-keyed `Orchestrator`, reusing the existing `ProcessSpawner`/`Process`/`SpawnSpec` primitives but keyed by `(site, proc)`. Each process runs as `sh -c 'exec <cmd> >> <log> 2>&1'` in the site root so the tracked PID is the real process and output is captured without changing `SpawnSpec`. The desktop layer adds commands + a 1s monitor event + Start-All/Stop-All/exit integration; the frontend adds a "⚙ N" row chip and a row-menu "Processes" modal.

**Tech Stack:** Rust (laralux-core: zero Tauri deps; laralux-desktop: Tauri 2), TypeScript (Vite strict, `noUnusedLocals`/`noUnusedParameters`).

## Global Constraints

- **laralux-core keeps ZERO Tauri dependencies.** No new crate dependency is needed.
- Procfile format: Foreman standard — `name: command` per line, at `<site.root>/Procfile`; `#` comments and blank lines ignored; `name` matches `[A-Za-z0-9_-]+`; malformed lines skipped (never abort the file); duplicate names keep the first.
- Process state vocabulary reuses the existing `crate::service::ServiceState` (Stopped/Starting/Running/Stopping/Crashed). **No new state enum.**
- Spawn shape: `SpawnSpec::new("sh").arg("-c").arg("exec <command> >> <log> 2>&1")`, `cwd = site root`, `env PATH = <managed_bin_dirs joined>:/usr/local/bin:<inherited PATH>`. Log file: `~/laralux/log/proc-<site>-<name>.log`.
- **No auto-restart** (a crashed proc stays Crashed until the user restarts — matches the service model). No `$PORT` assignment, no `name=N` concurrency (out of scope v1).
- Autostart persisted as `Config.proc_autostart: BTreeSet<String>` (site names), `#[serde(default)]`, default empty.
- Commits: **no `Co-Authored-By` trailer.** Work on `master` (direct commits, this session's convention).
- Reference patterns to mirror: `core/src/process.rs` (primitives), `core/src/orchestrator.rs` (`refresh` liveness loop), `core/src/service/mod.rs` (`SpawnSpec` builder), `src/ui/views/settings.ts` (handler-function + optimistic-update pattern), `src/ui/modals/domains.ts` (modal pattern).

---

### Task 1: Procfile parser (`core/src/procfile.rs`)

**Files:**
- Create: `core/src/procfile.rs`
- Modify: `core/src/lib.rs` (add `pub mod procfile;` near the other `pub mod` lines ~line 31; add a `pub use` near ~line 73)

**Interfaces:**
- Consumes: nothing (pure std).
- Produces: `ProcEntry { name: String, command: String }`, `parse_procfile(text: &str) -> Vec<ProcEntry>`, `read_procfile(site_root: &Path) -> Option<Vec<ProcEntry>>`.

- [ ] **Step 1: Write the parser file with tests**

Create `core/src/procfile.rs`:

```rust
use std::path::Path;

/// One declared process from a Procfile line `name: command`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcEntry {
    pub name: String,
    pub command: String,
}

/// Parse Foreman-style Procfile text. Lenient: blank lines and `#` comments are
/// ignored, malformed lines are skipped (never aborts), `name` must match
/// `[A-Za-z0-9_-]+`, and a duplicate name keeps the first occurrence.
pub fn parse_procfile(text: &str) -> Vec<ProcEntry> {
    let mut out: Vec<ProcEntry> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((raw_name, raw_cmd)) = trimmed.split_once(':') else {
            continue;
        };
        let name = raw_name.trim();
        let command = raw_cmd.trim();
        if name.is_empty() || command.is_empty() {
            continue;
        }
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            continue;
        }
        if out.iter().any(|e| e.name == name) {
            continue;
        }
        out.push(ProcEntry { name: name.to_string(), command: command.to_string() });
    }
    out
}

/// Read `<site_root>/Procfile`. `None` if the file is absent; `Some(entries)`
/// (possibly empty) otherwise.
pub fn read_procfile(site_root: &Path) -> Option<Vec<ProcEntry>> {
    match std::fs::read_to_string(site_root.join("Procfile")) {
        Ok(text) => Some(parse_procfile(&text)),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_commands_comments_blanks() {
        let text = "\n# a comment\nweb: php artisan serve\n\nqueue:  php artisan queue:work --tries=3\n";
        let e = parse_procfile(text);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0], ProcEntry { name: "web".into(), command: "php artisan serve".into() });
        // command keeps a colon after the first split; surrounding spaces trimmed.
        assert_eq!(e[1].name, "queue");
        assert_eq!(e[1].command, "php artisan queue:work --tries=3");
    }

    #[test]
    fn skips_malformed_and_duplicate_lines() {
        let text = "no colon here\nbad name!: cmd\nok: first\nok: second\n: empty name\nempty_cmd:   \n";
        let e = parse_procfile(text);
        // only `ok: first` survives (dup `ok` skipped, others malformed/empty)
        assert_eq!(e.len(), 1);
        assert_eq!(e[0], ProcEntry { name: "ok".into(), command: "first".into() });
    }

    #[test]
    fn read_procfile_absent_is_none() {
        let dir = std::env::temp_dir().join(format!("lara-proc-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_procfile(&dir).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_procfile_present_parses() {
        let dir = std::env::temp_dir().join(format!("lara-proc-yes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Procfile"), b"worker: sleep 1\n").unwrap();
        let e = read_procfile(&dir).unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].name, "worker");
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 2: Register the module in `core/src/lib.rs`**

Add near the other `pub mod` declarations (e.g. after `pub mod postgres_static;`):

```rust
pub mod procfile;
```

Add near the other `pub use` lines (e.g. after the `postgres_static` re-export):

```rust
pub use procfile::{parse_procfile, read_procfile, ProcEntry};
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p laralux-core procfile`
Expected: PASS (4 tests).

- [ ] **Step 4: Commit**

```bash
git add core/src/procfile.rs core/src/lib.rs
git commit -m "feat(core): Procfile parser (parse_procfile/read_procfile)"
```

---

### Task 2: SiteProcs supervisor (`core/src/site_procs.rs`)

**Files:**
- Create: `core/src/site_procs.rs`
- Modify: `core/src/lib.rs` (add `pub mod site_procs;` and a `pub use`)

**Interfaces:**
- Consumes: `crate::layout::managed_bin_dirs`, `crate::paths::LaraluxPaths`, `crate::process::{Process, ProcessSpawner}`, `crate::procfile::read_procfile` (Task 1), `crate::service::{ServiceState, SpawnSpec}`.
- Produces: `SiteProcs` with `new(paths, spawner)`, `start(site,root,name,command)`, `stop(site,name)`, `start_site(site,root)`, `stop_site(site)`, `stop_all()`, `refresh()`, `state_of(site,name) -> ServiceState`, `pid_of(site,name) -> Option<u32>`, `state_pairs() -> Vec<(String,String,ServiceState)>`; and `ProcStatus { site, name, command, state, pid }` (Serialize).

- [ ] **Step 1: Write the supervisor file with tests**

Create `core/src/site_procs.rs`:

```rust
use crate::layout::managed_bin_dirs;
use crate::paths::LaraluxPaths;
use crate::process::{Process, ProcessSpawner};
use crate::procfile::read_procfile;
use crate::service::{ServiceState, SpawnSpec};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

type Key = (String, String); // (site name, proc name)

/// One process's status for the UI (declared command + live state).
#[derive(Debug, Clone, Serialize)]
pub struct ProcStatus {
    pub site: String,
    pub name: String,
    pub command: String,
    pub state: ServiceState,
    pub pid: Option<u32>,
}

/// Supervises per-site Procfile processes. Independent of the ServiceKind-keyed
/// Orchestrator; reuses the same process primitives.
pub struct SiteProcs {
    paths: LaraluxPaths,
    spawner: Box<dyn ProcessSpawner>,
    handles: HashMap<Key, Box<dyn Process>>,
    states: HashMap<Key, ServiceState>,
}

impl SiteProcs {
    pub fn new(paths: LaraluxPaths, spawner: Box<dyn ProcessSpawner>) -> Self {
        Self { paths, spawner, handles: HashMap::new(), states: HashMap::new() }
    }

    /// PATH that prepends the managed tool bins + /usr/local/bin so `php`,
    /// `node`, `composer`, etc. resolve to the versions Laralux manages.
    fn proc_path_env(&self) -> String {
        let mut p = String::new();
        for d in managed_bin_dirs(&self.paths) {
            p.push_str(&d.display().to_string());
            p.push(':');
        }
        p.push_str("/usr/local/bin:");
        p.push_str(&std::env::var("PATH").unwrap_or_default());
        p
    }

    fn spawn_spec(&self, root: &Path, site: &str, name: &str, command: &str) -> SpawnSpec {
        let log = self.paths.log().join(format!("proc-{site}-{name}.log"));
        // `exec` so the tracked PID is the real worker (stop() signals it, not a
        // wrapper shell); the shell `>>` redirect captures stdout+stderr.
        let shell = format!("exec {command} >> {} 2>&1", log.display());
        SpawnSpec::new("sh")
            .arg("-c")
            .arg(shell)
            .cwd(root.to_path_buf())
            .env("PATH", self.proc_path_env())
    }

    /// Start one process. Idempotent: a no-op if a live handle already exists.
    pub fn start(&mut self, site: &str, root: &Path, name: &str, command: &str) -> std::io::Result<()> {
        let key = (site.to_string(), name.to_string());
        if let Some(h) = self.handles.get_mut(&key) {
            if h.is_alive() {
                return Ok(());
            }
        }
        std::fs::create_dir_all(self.paths.log())?;
        let spec = self.spawn_spec(root, site, name, command);
        match self.spawner.spawn(&spec) {
            Ok(handle) => {
                self.handles.insert(key.clone(), handle);
                self.states.insert(key, ServiceState::Running);
                Ok(())
            }
            Err(e) => {
                self.states.insert(key, ServiceState::Crashed);
                Err(e)
            }
        }
    }

    pub fn stop(&mut self, site: &str, name: &str) {
        let key = (site.to_string(), name.to_string());
        if let Some(mut h) = self.handles.remove(&key) {
            let _ = h.stop();
        }
        self.states.insert(key, ServiceState::Stopped);
    }

    /// Start every process declared in the site's Procfile.
    pub fn start_site(&mut self, site: &str, root: &Path) {
        if let Some(entries) = read_procfile(root) {
            for e in entries {
                let _ = self.start(site, root, &e.name, &e.command);
            }
        }
    }

    pub fn stop_site(&mut self, site: &str) {
        let keys: Vec<Key> = self.handles.keys().filter(|(s, _)| s == site).cloned().collect();
        for (s, n) in keys {
            self.stop(&s, &n);
        }
    }

    pub fn stop_all(&mut self) {
        let keys: Vec<Key> = self.handles.keys().cloned().collect();
        for (s, n) in keys {
            self.stop(&s, &n);
        }
    }

    /// Poll liveness: a handle that died unexpectedly becomes `Crashed` (handle
    /// dropped). Mirrors `Orchestrator::refresh`. No auto-restart.
    pub fn refresh(&mut self) {
        let mut running: Vec<Key> = Vec::new();
        let mut dead: Vec<Key> = Vec::new();
        for (key, h) in self.handles.iter_mut() {
            if h.is_alive() {
                running.push(key.clone());
            } else {
                dead.push(key.clone());
            }
        }
        for key in running {
            self.states.insert(key, ServiceState::Running);
        }
        for key in dead {
            self.handles.remove(&key);
            self.states.insert(key, ServiceState::Crashed);
        }
    }

    pub fn state_of(&self, site: &str, name: &str) -> ServiceState {
        self.states
            .get(&(site.to_string(), name.to_string()))
            .copied()
            .unwrap_or(ServiceState::Stopped)
    }

    pub fn pid_of(&self, site: &str, name: &str) -> Option<u32> {
        self.handles.get(&(site.to_string(), name.to_string())).map(|h| h.pid())
    }

    /// (site, name, state) for every tracked proc, sorted — used by the monitor
    /// for cheap change detection.
    pub fn state_pairs(&self) -> Vec<(String, String, ServiceState)> {
        let mut v: Vec<(String, String, ServiceState)> =
            self.states.iter().map(|((s, n), st)| (s.clone(), n.clone(), *st)).collect();
        v.sort_by(|a, b| (a.0.as_str(), a.1.as_str()).cmp(&(b.0.as_str(), b.1.as_str())));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::FakeSpawner;

    fn paths() -> LaraluxPaths {
        LaraluxPaths::new(std::env::temp_dir().join(format!("lara-sp-{}", std::process::id())))
    }

    #[test]
    fn start_records_shell_spec_with_cwd_and_path() {
        let spawner = FakeSpawner::new();
        let log = spawner.log();
        let mut sp = SiteProcs::new(paths(), Box::new(spawner));
        sp.start("blog", Path::new("/srv/blog"), "web", "php artisan serve").unwrap();
        let recorded = log.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        let spec = &recorded[0];
        assert_eq!(spec.program, "sh");
        assert_eq!(spec.args[0], "-c");
        assert!(spec.args[1].contains("exec php artisan serve"));
        assert!(spec.args[1].contains("proc-blog-web.log"));
        assert!(spec.args[1].contains(">>"));
        assert_eq!(spec.cwd.as_deref(), Some(Path::new("/srv/blog")));
        assert!(spec.env.iter().any(|(k, _)| k == "PATH"));
    }

    #[test]
    fn start_is_idempotent_for_live_handle() {
        let spawner = FakeSpawner::new();
        let log = spawner.log();
        let mut sp = SiteProcs::new(paths(), Box::new(spawner));
        sp.start("s", Path::new("/x"), "w", "sleep 1").unwrap();
        sp.start("s", Path::new("/x"), "w", "sleep 1").unwrap();
        assert_eq!(log.lock().unwrap().len(), 1);
        assert_eq!(sp.state_of("s", "w"), ServiceState::Running);
        assert!(sp.pid_of("s", "w").is_some());
    }

    #[test]
    fn start_site_starts_every_entry() {
        let dir = std::env::temp_dir().join(format!("lara-sp-site-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Procfile"), b"web: sleep 1\nqueue: sleep 1\n").unwrap();
        let spawner = FakeSpawner::new();
        let log = spawner.log();
        let mut sp = SiteProcs::new(paths(), Box::new(spawner));
        sp.start_site("blog", &dir);
        assert_eq!(log.lock().unwrap().len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stop_sets_stopped_and_drops_handle() {
        let spawner = FakeSpawner::new();
        let mut sp = SiteProcs::new(paths(), Box::new(spawner));
        sp.start("s", Path::new("/x"), "w", "sleep 1").unwrap();
        sp.stop("s", "w");
        assert_eq!(sp.state_of("s", "w"), ServiceState::Stopped);
        assert!(sp.pid_of("s", "w").is_none());
    }

    #[test]
    fn refresh_marks_dead_handle_crashed() {
        // A spawner whose process reports not-alive, to drive the Crashed path.
        struct DeadSpawner;
        struct DeadProc;
        impl Process for DeadProc {
            fn is_alive(&mut self) -> bool { false }
            fn stop(&mut self) -> std::io::Result<()> { Ok(()) }
            fn reload(&mut self) -> std::io::Result<()> { Ok(()) }
            fn pid(&self) -> u32 { 4242 }
        }
        impl ProcessSpawner for DeadSpawner {
            fn spawn(&self, _spec: &SpawnSpec) -> std::io::Result<Box<dyn Process>> {
                Ok(Box::new(DeadProc))
            }
        }
        let mut sp = SiteProcs::new(paths(), Box::new(DeadSpawner));
        sp.start("s", Path::new("/x"), "w", "sleep 1").unwrap();
        sp.refresh();
        assert_eq!(sp.state_of("s", "w"), ServiceState::Crashed);
        assert!(sp.pid_of("s", "w").is_none());
    }

    #[test]
    fn state_of_defaults_stopped() {
        let sp = SiteProcs::new(paths(), Box::new(FakeSpawner::new()));
        assert_eq!(sp.state_of("nope", "nope"), ServiceState::Stopped);
    }
}
```

- [ ] **Step 2: Register the module in `core/src/lib.rs`**

```rust
pub mod site_procs;
```

```rust
pub use site_procs::{ProcStatus, SiteProcs};
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p laralux-core site_procs`
Expected: PASS (6 tests).

- [ ] **Step 4: Commit**

```bash
git add core/src/site_procs.rs core/src/lib.rs
git commit -m "feat(core): SiteProcs supervisor for per-site Procfile processes"
```

---

### Task 3: Autostart config (`core/src/config.rs`)

**Files:**
- Modify: `core/src/config.rs` (`Config` struct ~line 32-46; `Default` impl ~line 55-59; add a test)

**Interfaces:**
- Consumes: nothing new (`BTreeSet` already imported at the top of config.rs).
- Produces: `Config.proc_autostart: BTreeSet<String>`.

- [ ] **Step 1: Add the field to `Config`**

In the `Config` struct, after the `symlinks` field, add:

```rust
    #[serde(default)]
    pub proc_autostart: BTreeSet<String>,
```

- [ ] **Step 2: Initialize it in `Default for Config`**

In `impl Default for Config`, add `proc_autostart: BTreeSet::new()` to the constructed struct (alongside `symlinks: BTreeSet::new()`):

```rust
        Self { tld: default_tld(), php_version: default_php(), services: ServicesConfig::default(), versions: BTreeMap::new(), symlinks: BTreeSet::new(), php_ini: crate::php_ini::PhpIniSettings::default(), proc_autostart: BTreeSet::new() }
```

- [ ] **Step 3: Add a test**

In `config.rs` `mod tests`, add:

```rust
    #[test]
    fn proc_autostart_defaults_empty_and_roundtrips() {
        let mut c = Config::default();
        assert!(c.proc_autostart.is_empty());
        c.proc_autostart.insert("blog".to_string());
        let toml = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&toml).unwrap();
        assert!(back.proc_autostart.contains("blog"));
        // old config without the field still loads
        let old: Config = toml::from_str("tld = \"dev\"\nphp_version = \"8.4\"\n").unwrap();
        assert!(old.proc_autostart.is_empty());
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p laralux-core config`
Expected: PASS (existing config tests + `proc_autostart_defaults_empty_and_roundtrips`).

- [ ] **Step 5: Commit**

```bash
git add core/src/config.rs
git commit -m "feat(core): Config.proc_autostart (per-site autostart flag)"
```

---

### Task 4: Desktop commands + state (`src-tauri/src/commands.rs`, `main.rs`)

**Files:**
- Modify: `src-tauri/src/commands.rs` (imports ~line 1-13; `AppState` ~line 24-29; `build_state` ~line 32-41; add the commands + view struct)
- Modify: `src-tauri/src/main.rs` (`invoke_handler` list ~line 48-77)

**Interfaces:**
- Consumes: `laralux_core::{SiteProcs, ProcStatus, read_procfile, list_all_sites, Config}`, `ServiceState`.
- Produces: Tauri commands `site_procs`, `start_site_proc`, `stop_site_proc`, `start_site_procs`, `stop_site_procs`, `set_site_autostart`, `site_proc_log_path`, `site_proc_counts`; `SiteProcsView { procs, autostart }`; `AppState.site_procs: Mutex<SiteProcs>`.

- [ ] **Step 1: Extend imports**

In the first `use laralux_core::{ ... };` block in `commands.rs`, add `ProcStatus`, `SiteProcs`, `read_procfile`, and (if not already present) `list_all_sites` is already imported. Add to the list: `ProcStatus, SiteProcs, read_procfile`. Also add at the top:

```rust
use std::collections::BTreeSet; // for set_site_autostart (BTreeSet ops) — only if needed; Config owns the set
use std::path::Path;
```

(If `BTreeSet` is not otherwise referenced, omit that line — `config.proc_autostart` methods don't require importing the type. Keep `use std::path::Path;`.)

- [ ] **Step 2: Add `site_procs` to `AppState`**

```rust
pub struct AppState {
    pub orch: Mutex<Orchestrator>,
    pub site_procs: Mutex<SiteProcs>,
    pub paths: LaraluxPaths,
    pub tld: String,
    pub starting: AtomicBool,
}
```

- [ ] **Step 3: Build it in `build_state`**

In `build_state`, after constructing `orch`, add and include it in the returned struct:

```rust
    let orch = Orchestrator::new(paths.clone(), build_services(&config, &paths), Box::new(RealSpawner));
    let site_procs = SiteProcs::new(paths.clone(), Box::new(RealSpawner));
    AppState { orch: Mutex::new(orch), site_procs: Mutex::new(site_procs), paths, tld: config.tld, starting: AtomicBool::new(false) }
```

- [ ] **Step 4: Add the view struct + helper + commands**

Append to `commands.rs`:

```rust
/// View-model for the Processes modal: the site's declared procs (merged with
/// live state) plus its autostart flag.
#[derive(serde::Serialize)]
pub struct SiteProcsView {
    pub procs: Vec<ProcStatus>,
    pub autostart: bool,
}

/// Build the view: refresh liveness, merge the parsed Procfile entries with
/// current state, and read the autostart flag. Caller holds the SiteProcs lock.
fn site_procs_view(sp: &mut SiteProcs, paths: &LaraluxPaths, name: &str, root: &str) -> SiteProcsView {
    sp.refresh();
    let entries = read_procfile(Path::new(root)).unwrap_or_default();
    let procs = entries
        .into_iter()
        .map(|e| ProcStatus {
            site: name.to_string(),
            name: e.name.clone(),
            command: e.command,
            state: sp.state_of(name, &e.name),
            pid: sp.pid_of(name, &e.name),
        })
        .collect();
    let autostart = Config::load(&paths.config_file())
        .unwrap_or_default()
        .proc_autostart
        .contains(name);
    SiteProcsView { procs, autostart }
}

#[tauri::command]
pub fn site_procs(state: tauri::State<AppState>, name: String, root: String) -> Result<SiteProcsView, String> {
    let mut sp = state.site_procs.lock().map_err(lock_err)?;
    Ok(site_procs_view(&mut sp, &state.paths, &name, &root))
}

#[tauri::command]
pub fn start_site_proc(state: tauri::State<AppState>, name: String, root: String, proc: String) -> Result<SiteProcsView, String> {
    let cmd = read_procfile(Path::new(&root))
        .unwrap_or_default()
        .into_iter()
        .find(|e| e.name == proc)
        .map(|e| e.command);
    let mut sp = state.site_procs.lock().map_err(lock_err)?;
    if let Some(c) = cmd {
        let _ = sp.start(&name, Path::new(&root), &proc, &c);
    }
    Ok(site_procs_view(&mut sp, &state.paths, &name, &root))
}

#[tauri::command]
pub fn stop_site_proc(state: tauri::State<AppState>, name: String, root: String, proc: String) -> Result<SiteProcsView, String> {
    let mut sp = state.site_procs.lock().map_err(lock_err)?;
    sp.stop(&name, &proc);
    Ok(site_procs_view(&mut sp, &state.paths, &name, &root))
}

#[tauri::command]
pub fn start_site_procs(state: tauri::State<AppState>, name: String, root: String) -> Result<SiteProcsView, String> {
    let mut sp = state.site_procs.lock().map_err(lock_err)?;
    sp.start_site(&name, Path::new(&root));
    Ok(site_procs_view(&mut sp, &state.paths, &name, &root))
}

#[tauri::command]
pub fn stop_site_procs(state: tauri::State<AppState>, name: String, root: String) -> Result<SiteProcsView, String> {
    let mut sp = state.site_procs.lock().map_err(lock_err)?;
    sp.stop_site(&name);
    Ok(site_procs_view(&mut sp, &state.paths, &name, &root))
}

#[tauri::command]
pub fn set_site_autostart(state: tauri::State<AppState>, name: String, enabled: bool) -> Result<bool, String> {
    let mut config = Config::load(&state.paths.config_file()).unwrap_or_default();
    if enabled {
        config.proc_autostart.insert(name);
    } else {
        config.proc_autostart.remove(&name);
    }
    config.save(&state.paths.config_file()).map_err(|e| e.to_string())?;
    Ok(enabled)
}

#[tauri::command]
pub fn site_proc_log_path(state: tauri::State<AppState>, name: String, proc: String) -> Result<String, String> {
    Ok(state.paths.log().join(format!("proc-{name}-{proc}.log")).display().to_string())
}

#[tauri::command]
pub fn site_proc_counts(state: tauri::State<AppState>) -> Result<std::collections::BTreeMap<String, usize>, String> {
    let (sites, _warnings) = list_all_sites(&state.paths, &state.tld).map_err(|e| e.to_string())?;
    let mut out = std::collections::BTreeMap::new();
    for s in sites {
        if let Some(entries) = read_procfile(&s.root) {
            if !entries.is_empty() {
                out.insert(s.name, entries.len());
            }
        }
    }
    Ok(out)
}
```

- [ ] **Step 5: Register the commands in `main.rs`**

In the `tauri::generate_handler![ ... ]` list, add:

```rust
            commands::site_procs,
            commands::start_site_proc,
            commands::stop_site_proc,
            commands::start_site_procs,
            commands::stop_site_procs,
            commands::set_site_autostart,
            commands::site_proc_log_path,
            commands::site_proc_counts,
```

- [ ] **Step 6: Build the desktop crate**

Run: `cargo build -p laralux-desktop`
Expected: Finished (0 errors).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(desktop): site Procfile commands + SiteProcs in AppState"
```

---

### Task 5: Desktop monitor event + Start-All/Stop-All/exit integration (`src-tauri/src/main.rs`, `commands.rs`)

**Files:**
- Modify: `src-tauri/src/main.rs` (monitor thread ~line 165-211; tray "quit" handler ~line 150-157; `ExitRequested` ~line 249-257)
- Modify: `src-tauri/src/commands.rs` (`run_full_start` ~line 60+; `stack_stop_all` ~line 112+)

**Interfaces:**
- Consumes: `AppState.site_procs` (Task 4), `SiteProcs::{refresh, state_pairs, start_site, stop_all}` (Task 2), `Config.proc_autostart` (Task 3), `list_all_sites`.
- Produces: a `site-procs-changed` event (emitted change-only); autostart procs started by Start All; all procs stopped by Stop All and on exit.

- [ ] **Step 1: Emit `site-procs-changed` from the monitor thread**

In `main.rs`, inside the existing ~1s monitor loop (the `std::thread::spawn` that polls `state.orch`), after the existing service-snapshot/tray block, add a site-procs poll. First, add a `last_procs` accumulator next to the loop's other `last_*` bindings (near `let mut last_all_running ...`):

```rust
                    let mut last_procs: Vec<(String, String, laralux_core::ServiceState)> = Vec::new();
```

Then, inside the loop body (after the tray/toggle update, before the closing `}` of the loop), add:

```rust
                        if let Ok(mut sp) = state.site_procs.lock() {
                            sp.refresh();
                            let pairs = sp.state_pairs();
                            if pairs != last_procs {
                                let _ = handle.emit("site-procs-changed", ());
                                last_procs = pairs;
                            }
                        }
```

- [ ] **Step 2: Start autostart sites in `run_full_start`**

In `commands.rs`, at the END of `run_full_start` (immediately before it returns its messages `Vec<String>`), add:

```rust
    // Start each autostart site's Procfile processes once the stack is up.
    let cfg = Config::load(&state.paths.config_file()).unwrap_or_default();
    if !cfg.proc_autostart.is_empty() {
        if let Ok((sites, _warnings)) = list_all_sites(&state.paths, &state.tld) {
            if let Ok(mut sp) = state.site_procs.lock() {
                for s in &sites {
                    if cfg.proc_autostart.contains(&s.name) {
                        sp.start_site(&s.name, &s.root);
                    }
                }
            }
        }
    }
```

(If `run_full_start` builds a named messages vector and returns it, place this block right before that `return`/trailing expression; it does not modify the messages.)

- [ ] **Step 3: Stop all site procs in `stack_stop_all`**

In `commands.rs` `stack_stop_all`, after stopping the orchestrator (`orch.stop_all()`), add a stop of site procs. Insert before returning the snapshot:

```rust
    if let Ok(mut sp) = state.site_procs.lock() {
        sp.stop_all();
    }
```

- [ ] **Step 4: Stop site procs on quit + exit in `main.rs`**

In the tray `"quit"` handler, after `orch.stop_all();` add:

```rust
                            if let Ok(mut sp) = state.site_procs.lock() {
                                sp.stop_all();
                            }
```

In the `RunEvent::ExitRequested` block at the bottom of `main.rs`, after `orch.stop_all();` add the same:

```rust
                    if let Ok(mut sp) = state.site_procs.lock() {
                        sp.stop_all();
                    }
```

- [ ] **Step 5: Build the desktop crate**

Run: `cargo build -p laralux-desktop`
Expected: Finished (0 errors).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/main.rs src-tauri/src/commands.rs
git commit -m "feat(desktop): site-procs-changed monitor + Start-All/Stop-All/exit wiring"
```

---

### Task 6: Frontend IPC + state (`types.ts`, `commands.ts`, `events.ts`, `state.ts`)

**Files:**
- Modify: `src/ipc/types.ts` (add `ProcStatus`, `SiteProcsView`)
- Modify: `src/ipc/commands.ts` (add 8 wrappers)
- Modify: `src/ipc/events.ts` (add `onSiteProcsChanged`)
- Modify: `src/state.ts` (add `procCounts`, `procModal`, `siteProcs`; extend `modal` union)

**Interfaces:**
- Consumes: backend commands from Tasks 4-5 and the `site-procs-changed` event.
- Produces: `siteProcs/startSiteProc/stopSiteProc/startSiteProcs/stopSiteProcs/setSiteAutostart/siteProcLogPath/siteProcCounts`, `onSiteProcsChanged`, and `state.procCounts/procModal/siteProcs`.

- [ ] **Step 1: Add types in `src/ipc/types.ts`**

After the `ServiceStatus` interface, add (reusing the existing `ServiceState` union):

```ts
export interface ProcStatus {
  site: string;
  name: string;
  command: string;
  state: ServiceState;
  pid: number | null;
}

export interface SiteProcsView {
  procs: ProcStatus[];
  autostart: boolean;
}
```

- [ ] **Step 2: Add command wrappers in `src/ipc/commands.ts`**

Add (mirroring the existing `invoke` wrappers; import `SiteProcsView` from `./types`):

```ts
export const siteProcs = (name: string, root: string): Promise<SiteProcsView> =>
  invoke("site_procs", { name, root });

export const startSiteProc = (name: string, root: string, proc: string): Promise<SiteProcsView> =>
  invoke("start_site_proc", { name, root, proc });

export const stopSiteProc = (name: string, root: string, proc: string): Promise<SiteProcsView> =>
  invoke("stop_site_proc", { name, root, proc });

export const startSiteProcs = (name: string, root: string): Promise<SiteProcsView> =>
  invoke("start_site_procs", { name, root });

export const stopSiteProcs = (name: string, root: string): Promise<SiteProcsView> =>
  invoke("stop_site_procs", { name, root });

export const setSiteAutostart = (name: string, enabled: boolean): Promise<boolean> =>
  invoke("set_site_autostart", { name, enabled });

export const siteProcLogPath = (name: string, proc: string): Promise<string> =>
  invoke("site_proc_log_path", { name, proc });

export const siteProcCounts = (): Promise<Record<string, number>> =>
  invoke("site_proc_counts", {});
```

(Match the file's existing import of `invoke` and `type` imports; add `SiteProcsView` to the `import type { ... } from "./types";` line.)

- [ ] **Step 3: Add the event subscription in `src/ipc/events.ts`**

```ts
/**
 * Subscribe to "site-procs-changed" — emitted (change-only) by the monitor when
 * any tracked per-site process changes state. Payload is empty; the handler
 * re-fetches the open Processes modal.
 */
export const onSiteProcsChanged = (cb: () => void): Promise<UnlistenFn> =>
  listen("site-procs-changed", () => cb());
```

- [ ] **Step 4: Extend `src/state.ts`**

Add `SiteProcsView` to the `import type { ... } from "./ipc/types";` line. Extend the `modal` union to include `"procs"`:

```ts
  modal: null | "newsite" | "linksite" | "proxy" | "domains" | "deletesite" | "procs" | ToolModalState;
```

Add three fields to the `AppState` interface (near `sites`/`siteDomains`):

```ts
  procCounts: Record<string, number>;
  procModal: { name: string; root: string } | null;
  siteProcs: SiteProcsView | null;
```

And initialize them in the exported `state` object (near `sites: []`):

```ts
  procCounts: {},
  procModal: null,
  siteProcs: null,
```

- [ ] **Step 5: Build the frontend**

Run: `npm run build`
Expected: `✓ built` with no TypeScript errors.

- [ ] **Step 6: Commit**

```bash
git add src/ipc/types.ts src/ipc/commands.ts src/ipc/events.ts src/state.ts
git commit -m "feat(ui): IPC + state for per-site Procfile processes"
```

---

### Task 7: Frontend UI — Processes modal, row chip, wiring (`modals/procs.ts`, `sites.ts`, `events.ts`, `render.ts`, `main.ts`)

**Files:**
- Create: `src/ui/modals/procs.ts` (modal render + handler functions)
- Modify: `src/ui/views/sites.ts` (row chip + row-menu "Processes" item)
- Modify: `src/ui/events.ts` (dispatch the new `data-action`s)
- Modify: `src/ui/render.ts` (render `procsModal()` when `state.modal === "procs"`)
- Modify: `src/main.ts` (subscribe `onSiteProcsChanged`; load `siteProcCounts` on boot + on sites-changed)

**Interfaces:**
- Consumes: Task 6's IPC wrappers/state, `META`/`ServiceState` styling from `src/ui/constants.ts`, `esc` from `src/ui/util`, `I` icons.
- Produces: `procsModal()`, `openProcs`, `closeProcs`, `procStart`, `procStop`, `procStartAll`, `procStopAll`, `procToggleAutostart`, `procLogs`, `loadProcCounts`.

- [ ] **Step 1: Create `src/ui/modals/procs.ts`**

```ts
import { state } from "../../state";
import { esc } from "../util";
import { render } from "../render";
import { toast } from "../toast";
import { META } from "../constants";
import {
  siteProcs, startSiteProc, stopSiteProc, startSiteProcs, stopSiteProcs,
  setSiteAutostart, siteProcLogPath,
} from "../../ipc/commands";

export async function openProcs(name: string, root: string): Promise<void> {
  state.procModal = { name, root };
  state.siteProcs = null;
  state.modal = "procs";
  render();
  try {
    state.siteProcs = await siteProcs(name, root);
  } catch (e) {
    toast({ type: "error", title: "Couldn't load processes", msg: String(e) });
  }
  render();
}

export function closeProcs(): void {
  state.modal = null;
  state.procModal = null;
  state.siteProcs = null;
  render();
}

/** Re-fetch the open modal (used by the live event + after actions). */
export async function refreshProcs(): Promise<void> {
  if (!state.procModal) return;
  const { name, root } = state.procModal;
  try {
    state.siteProcs = await siteProcs(name, root);
    render();
  } catch { /* modal may have closed */ }
}

export async function procStart(proc: string): Promise<void> {
  if (!state.procModal) return;
  const { name, root } = state.procModal;
  try { state.siteProcs = await startSiteProc(name, root, proc); }
  catch (e) { toast({ type: "error", title: proc + " start failed", msg: String(e) }); }
  render();
}

export async function procStop(proc: string): Promise<void> {
  if (!state.procModal) return;
  const { name, root } = state.procModal;
  try { state.siteProcs = await stopSiteProc(name, root, proc); }
  catch (e) { toast({ type: "error", title: proc + " stop failed", msg: String(e) }); }
  render();
}

export async function procStartAll(): Promise<void> {
  if (!state.procModal) return;
  const { name, root } = state.procModal;
  try { state.siteProcs = await startSiteProcs(name, root); }
  catch (e) { toast({ type: "error", title: "Start all failed", msg: String(e) }); }
  render();
}

export async function procStopAll(): Promise<void> {
  if (!state.procModal) return;
  const { name, root } = state.procModal;
  try { state.siteProcs = await stopSiteProcs(name, root); }
  catch (e) { toast({ type: "error", title: "Stop all failed", msg: String(e) }); }
  render();
}

export async function procToggleAutostart(): Promise<void> {
  if (!state.procModal || !state.siteProcs) return;
  const { name } = state.procModal;
  const next = !state.siteProcs.autostart;
  state.siteProcs = { ...state.siteProcs, autostart: next };
  render();
  try { await setSiteAutostart(name, next); }
  catch (e) {
    state.siteProcs = { ...state.siteProcs, autostart: !next };
    toast({ type: "error", title: "Couldn't change autostart", msg: String(e) });
    render();
  }
}

export async function procLogs(proc: string): Promise<void> {
  if (!state.procModal) return;
  const { name } = state.procModal;
  try {
    const path = await siteProcLogPath(name, proc);
    toast({ type: "info", sticky: true, title: proc + " logs", details: ["Log file: " + path, "or: tail -f " + path] });
  } catch (e) {
    toast({ type: "error", title: "No log path", msg: String(e) });
  }
}

export function procsModal(): string {
  const m = state.procModal;
  const view = state.siteProcs;
  if (!m) return "";
  const autostart = view?.autostart ?? false;
  const rows = !view
    ? '<div class="proc-empty">Loading…</div>'
    : view.procs.length === 0
      ? '<div class="proc-empty">No Procfile in this site’s folder.</div>'
      : view.procs.map((p) => {
          const meta = META[p.state] || META.Stopped;
          const running = p.state === "Running" || p.state === "Starting";
          const btn = running
            ? '<button class="btn-sm" data-action="proc-stop" data-proc="' + esc(p.name) + '">Stop</button>'
            : '<button class="btn-sm primary" data-action="proc-start" data-proc="' + esc(p.name) + '">Start</button>';
          return (
            '<div class="proc-row"><div class="proc-info">' +
            '<div class="proc-name"><span class="dot bgc-' + meta.cls + '"></span>' + esc(p.name) + "</div>" +
            '<code class="proc-cmd">' + esc(p.command) + "</code></div>" +
            '<div class="proc-actions">' + btn +
            '<button class="btn-xs" data-action="proc-logs" data-proc="' + esc(p.name) + '">Logs</button></div></div>'
          );
        }).join("");
  return (
    '<div class="modal-backdrop" data-action="close-procs"><div class="modal" data-stop>' +
    '<div class="modal-head"><h2>Processes — ' + esc(m.name) + "</h2>" +
    '<button class="icon-btn" data-action="close-procs" aria-label="Close">×</button></div>' +
    '<div class="modal-body">' +
    '<div class="proc-toolbar">' +
    '<button class="' + (autostart ? "toggle-on" : "toggle-off") + '" data-action="proc-autostart" aria-pressed="' + autostart + '"><span class="knob"></span></button>' +
    '<span class="proc-toolbar-label">Autostart with “Start All”</span><span class="spacer"></span>' +
    '<button class="btn-sm" data-action="proc-start-all">Start all</button>' +
    '<button class="btn-sm" data-action="proc-stop-all">Stop all</button></div>' +
    rows +
    "</div></div></div>"
  );
}
```

- [ ] **Step 2: Add the row chip + row-menu item in `src/ui/views/sites.ts`**

In the row-menu block (the kebab menu items, near the `Delete` item), add a "Processes" item — only when the site has a Procfile:

```ts
            (state.procCounts[s.name] ? '<button class="row-menu-item" data-action="open-procs" data-name="' + esc(s.name) + '" data-root="' + esc(s.root) + '">' + I.terminal + "Processes</button>" : "") +
```

And add a small chip near the site URL/subtitle (in the `site-sub` area) showing the count when present:

```ts
            (state.procCounts[s.name] ? '<span class="proc-chip" title="' + state.procCounts[s.name] + ' process(es) in Procfile">⚙ ' + state.procCounts[s.name] + "</span>" : "") +
```

(Place the chip inside the existing `site-sub` div alongside the URL link.)

- [ ] **Step 3: Dispatch the new actions in `src/ui/events.ts`**

Import the handlers at the top of `events.ts`:

```ts
import { openProcs, closeProcs, procStart, procStop, procStartAll, procStopAll, procToggleAutostart, procLogs } from "./modals/procs";
```

In the delegated `data-action` switch, add cases (mirroring the existing `data-*` attribute reads):

```ts
    case "open-procs": openProcs(t.dataset.name!, t.dataset.root!); break;
    case "close-procs": closeProcs(); break;
    case "proc-start": procStart(t.dataset.proc!); break;
    case "proc-stop": procStop(t.dataset.proc!); break;
    case "proc-start-all": procStartAll(); break;
    case "proc-stop-all": procStopAll(); break;
    case "proc-autostart": procToggleAutostart(); break;
    case "proc-logs": procLogs(t.dataset.proc!); break;
```

(Use the same element/dataset accessor the file already uses — if it reads `el.getAttribute("data-proc")` instead of `dataset`, match that style. The `close-procs` backdrop click and the `data-stop` inner guard mirror the existing modal close pattern.)

- [ ] **Step 4: Render the modal in `src/ui/render.ts`**

Add the import near the other modal imports:

```ts
import { procsModal } from "./modals/procs";
```

Add to the modal chain (after the `deletesite` branch):

```ts
    : state.modal === "procs" ? procsModal()
```

- [ ] **Step 5: Wire the live event + counts in `src/main.ts`**

Add to the events import:

```ts
import { onServicesChanged, onSitesChanged, onDownloadProgress, onSiteProcsChanged } from "./ipc/events";
```

Add a counts loader + subscriptions (near the existing `onSitesChanged` boot block). Import `siteProcCounts` from `./ipc/commands` and `refreshProcs` from `./ui/modals/procs`:

```ts
async function loadProcCounts(): Promise<void> {
  try { state.procCounts = await siteProcCounts(); render(); } catch { /* ignore */ }
}
loadProcCounts();
onSitesChanged(() => { loadProcCounts(); });
onSiteProcsChanged(() => { refreshProcs(); });
```

(Keep the existing `onSitesChanged` site-list refresh; just also call `loadProcCounts()` there, or add the second subscription as shown.)

- [ ] **Step 6: Build the frontend**

Run: `npm run build`
Expected: `✓ built` with no TypeScript errors. (If `tsc` flags an unused import or a missing `data-*` access style, align with the file's conventions and rebuild.)

- [ ] **Step 7: Commit**

```bash
git add src/ui/modals/procs.ts src/ui/views/sites.ts src/ui/events.ts src/ui/render.ts src/main.ts
git commit -m "feat(ui): Processes modal + site Procfile row chip and wiring"
```

---

## Self-Review

- **Spec coverage:** §2 parser → Task 1; §3 supervisor → Task 2; §4 autostart config → Task 3; §5 desktop commands → Task 4, monitor/lifecycle → Task 5; §6 frontend IPC/state → Task 6, UI (chip + modal) → Task 7; §9 tests embedded in Tasks 1-3 + manual smoke. §7 data-flow and §8 error handling are realized across Tasks 2 (no auto-restart, crash→Crashed), 4 (empty modal when no Procfile), 7 (empty state, View logs).
- **Placeholder scan:** none — every code step is complete. Two intentional "match the file's existing accessor style" notes (events.ts dataset vs getAttribute; run_full_start return placement) are conventions to follow, not missing code.
- **Type consistency:** `ProcStatus { site, name, command, state, pid }` identical in core (`site_procs.rs`) and TS (`types.ts`); `SiteProcsView { procs, autostart }` identical in `commands.rs` and `types.ts`. Command names match between `main.rs` registration, `commands.rs` definitions, and `commands.ts` wrappers (`site_procs`, `start_site_proc`, `stop_site_proc`, `start_site_procs`, `stop_site_procs`, `set_site_autostart`, `site_proc_log_path`, `site_proc_counts`). `ServiceState` reused everywhere (no parallel enum). `state.procModal`/`state.siteProcs`/`state.procCounts` defined in Task 6 and consumed in Task 7. Log filename `proc-<site>-<name>.log` identical in `site_procs.rs::spawn_spec` and `commands.rs::site_proc_log_path`.
- **Compile-order note:** Task 2 depends on Task 1 (`read_procfile`); Task 4 depends on Tasks 1-3; Task 5 depends on Task 4 (`AppState.site_procs`); Task 7 depends on Task 6. Each task's stated verification (core test / desktop build / npm build) is green at its own boundary. Minor CSS classes referenced by the modal (`proc-row`, `proc-chip`, `proc-toolbar`, etc.) reuse existing modal/toggle styling conventions; unstyled-but-functional is acceptable for v1 (the final review can note any polish), since `tsc`/build do not depend on CSS.
