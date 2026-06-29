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
