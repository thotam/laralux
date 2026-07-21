use crate::layout::managed_bin_dirs;
use crate::paths::LaraluxPaths;
use crate::process::{Process, ProcessSpawner};
use crate::procfile::read_procfile;
use crate::service::{ServiceState, SpawnSpec};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

type Key = (String, String); // (site name, proc name)

/// One process's status for the UI (declared command + live state).
#[derive(Debug, Clone, Serialize)]
pub struct ProcStatus {
    pub site: String,
    pub name: String,
    pub command: String,
    pub state: ServiceState,
    pub pid: Option<u32>,
    /// Consecutive restart failures; 0 when healthy. Lets the UI tell "still
    /// retrying" apart from "gave up".
    pub failures: u32,
}

/// Consecutive deaths before we stop resurrecting a process.
pub const MAX_RESTARTS: u32 = 5;
/// Staying up this long means the earlier deaths were a blip, not a crash loop.
const STABLE_AFTER: Duration = Duration::from_secs(30);

/// How long to wait before retry number `failures` (1-indexed). The 5th never
/// happens — `MAX_RESTARTS` gives up first.
fn backoff_for(failures: u32) -> Duration {
    match failures {
        1 => Duration::from_secs(1),
        2 => Duration::from_secs(5),
        3 => Duration::from_secs(15),
        _ => Duration::from_secs(30),
    }
}

/// Everything needed to resurrect a process, plus its backoff state.
struct Supervised {
    root: PathBuf,
    command: String,
    failures: u32,
    started_at: Instant,
    next_attempt_at: Option<Instant>,
    /// false once the user stopped it by hand, or once we gave up.
    supervised: bool,
}

/// Supervises per-site Procfile processes. Independent of the ServiceKind-keyed
/// Orchestrator; reuses the same process primitives.
pub struct SiteProcs {
    paths: LaraluxPaths,
    spawner: Box<dyn ProcessSpawner>,
    handles: HashMap<Key, Box<dyn Process>>,
    states: HashMap<Key, ServiceState>,
    supervision: HashMap<Key, Supervised>,
}

impl SiteProcs {
    pub fn new(paths: LaraluxPaths, spawner: Box<dyn ProcessSpawner>) -> Self {
        Self {
            paths,
            spawner,
            handles: HashMap::new(),
            states: HashMap::new(),
            supervision: HashMap::new(),
        }
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
                self.states.insert(key.clone(), ServiceState::Running);
                // Starting by hand wipes the slate: a fresh set of retries. This is
                // also the only place root/command are retained — without them a
                // dead process could not be respawned at all.
                self.supervision.insert(
                    key,
                    Supervised {
                        root: root.to_path_buf(),
                        command: command.to_string(),
                        failures: 0,
                        started_at: Instant::now(),
                        next_attempt_at: None,
                        supervised: true,
                    },
                );
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
        // A deliberate stop must never be undone by the supervisor.
        if let Some(s) = self.supervision.get_mut(&key) {
            s.supervised = false;
            s.next_attempt_at = None;
            s.failures = 0;
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

    /// Poll liveness and resurrect what died. Called from the app's monitor loop.
    pub fn refresh(&mut self) {
        self.tick_at(Instant::now());
    }

    /// The supervision tick, split out from `refresh()` so tests can feed
    /// synthetic instants instead of sleeping through real backoff windows.
    pub fn tick_at(&mut self, now: Instant) {
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
            // Up long enough → the earlier deaths were a blip, not a crash loop,
            // so a later failure gets a full set of retries again.
            if let Some(s) = self.supervision.get_mut(&key) {
                if s.failures > 0 && now.duration_since(s.started_at) >= STABLE_AFTER {
                    s.failures = 0;
                }
            }
            self.states.insert(key, ServiceState::Running);
        }

        for key in dead {
            self.handles.remove(&key);
            if let Some(s) = self.supervision.get_mut(&key) {
                if s.supervised {
                    s.failures += 1;
                    if s.failures >= MAX_RESTARTS {
                        s.supervised = false;
                        s.next_attempt_at = None;
                    } else {
                        s.next_attempt_at = Some(now + backoff_for(s.failures));
                    }
                }
            }
            self.states.insert(key, ServiceState::Crashed);
        }

        // Respawn whatever is due. Deliberately regardless of exit code: a
        // `queue:work` exits 0 on `queue:restart` and must come back.
        let due: Vec<Key> = self
            .supervision
            .iter()
            .filter(|(k, s)| {
                s.supervised
                    && !self.handles.contains_key(*k)
                    && s.next_attempt_at.map(|t| now >= t).unwrap_or(false)
            })
            .map(|(k, _)| k.clone())
            .collect();

        for key in due {
            let Some((root, command)) = self
                .supervision
                .get(&key)
                .map(|s| (s.root.clone(), s.command.clone()))
            else {
                continue;
            };
            let spec = self.spawn_spec(&root, &key.0, &key.1, &command);
            match self.spawner.spawn(&spec) {
                Ok(handle) => {
                    self.handles.insert(key.clone(), handle);
                    self.states.insert(key.clone(), ServiceState::Running);
                    if let Some(s) = self.supervision.get_mut(&key) {
                        s.started_at = now;
                        s.next_attempt_at = None;
                    }
                }
                Err(_) => {
                    // A failed respawn counts like any other death.
                    if let Some(s) = self.supervision.get_mut(&key) {
                        s.failures += 1;
                        if s.failures >= MAX_RESTARTS {
                            s.supervised = false;
                            s.next_attempt_at = None;
                        } else {
                            s.next_attempt_at = Some(now + backoff_for(s.failures));
                        }
                    }
                    self.states.insert(key, ServiceState::Crashed);
                }
            }
        }
    }

    /// Consecutive failures for a process; 0 when healthy or never started.
    pub fn failures_of(&self, site: &str, name: &str) -> u32 {
        self.supervision
            .get(&(site.to_string(), name.to_string()))
            .map(|s| s.failures)
            .unwrap_or(0)
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

    // ---- supervision -------------------------------------------------------

    use crate::process::{Process, ProcessSpawner};
    use crate::service::SpawnSpec;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    struct ControlledProc {
        alive: Arc<AtomicBool>,
    }
    impl Process for ControlledProc {
        fn is_alive(&mut self) -> bool {
            self.alive.load(Ordering::SeqCst)
        }
        fn stop(&mut self) -> std::io::Result<()> {
            self.alive.store(false, Ordering::SeqCst);
            Ok(())
        }
        fn reload(&mut self) -> std::io::Result<()> {
            Ok(())
        }
        fn pid(&self) -> u32 {
            7
        }
    }

    /// Spawns processes whose liveness the test drives, and counts spawns so a
    /// test can tell a respawn from a no-op tick.
    struct ControlledSpawner {
        alive: Arc<AtomicBool>,
        spawns: Arc<AtomicUsize>,
    }
    impl ProcessSpawner for ControlledSpawner {
        fn spawn(&self, _spec: &SpawnSpec) -> std::io::Result<Box<dyn Process>> {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(ControlledProc { alive: self.alive.clone() }))
        }
    }

    fn supervised() -> (SiteProcs, Arc<AtomicBool>, Arc<AtomicUsize>) {
        let alive = Arc::new(AtomicBool::new(false)); // dead on arrival by default
        let spawns = Arc::new(AtomicUsize::new(0));
        let sp = SiteProcs::new(
            paths(),
            Box::new(ControlledSpawner { alive: alive.clone(), spawns: spawns.clone() }),
        );
        (sp, alive, spawns)
    }

    #[test]
    fn dead_proc_is_retried_with_backoff_then_given_up() {
        let (mut sp, _alive, spawns) = supervised();
        sp.start("site", Path::new("/tmp"), "web", "true").unwrap();
        assert_eq!(spawns.load(Ordering::SeqCst), 1);

        let mut at = Instant::now();
        for (i, wait) in [1u64, 5, 15, 30].iter().enumerate() {
            sp.tick_at(at); // notice the death
            assert_eq!(sp.failures_of("site", "web"), i as u32 + 1);

            sp.tick_at(at + Duration::from_secs(*wait) - Duration::from_millis(1));
            assert_eq!(spawns.load(Ordering::SeqCst), i + 1, "chưa tới hạn thì chưa respawn");

            at += Duration::from_secs(*wait);
            sp.tick_at(at); // due → respawn
            assert_eq!(spawns.load(Ordering::SeqCst), i + 2, "tới hạn thì phải respawn");
            at += Duration::from_millis(1);
        }

        // Lần chết thứ 5 → bỏ cuộc.
        sp.tick_at(at);
        assert_eq!(sp.failures_of("site", "web"), 5);
        assert_eq!(sp.state_of("site", "web"), ServiceState::Crashed);
        let after_giveup = spawns.load(Ordering::SeqCst);
        sp.tick_at(at + Duration::from_secs(3600));
        assert_eq!(
            spawns.load(Ordering::SeqCst),
            after_giveup,
            "đã bỏ cuộc thì không bao giờ respawn nữa"
        );
    }

    #[test]
    fn staying_alive_long_enough_resets_the_failure_counter() {
        let (mut sp, alive, _spawns) = supervised();
        sp.start("site", Path::new("/tmp"), "web", "true").unwrap();

        let t = Instant::now();
        sp.tick_at(t); // chết lần 1
        assert_eq!(sp.failures_of("site", "web"), 1);

        alive.store(true, Ordering::SeqCst); // lần respawn tới sẽ sống
        let respawned = t + Duration::from_secs(1);
        sp.tick_at(respawned);
        assert_eq!(sp.state_of("site", "web"), ServiceState::Running);

        // Chưa đủ 30s thì vẫn giữ bộ đếm.
        sp.tick_at(respawned + Duration::from_secs(29));
        assert_eq!(sp.failures_of("site", "web"), 1);

        // Sống đủ 30s → coi như đã ổn định.
        sp.tick_at(respawned + Duration::from_secs(30));
        assert_eq!(sp.failures_of("site", "web"), 0);
    }

    #[test]
    fn manual_stop_is_never_respawned() {
        let (mut sp, _alive, spawns) = supervised();
        sp.start("site", Path::new("/tmp"), "web", "true").unwrap();
        sp.stop("site", "web");
        let before = spawns.load(Ordering::SeqCst);

        sp.tick_at(Instant::now() + Duration::from_secs(600));

        assert_eq!(sp.state_of("site", "web"), ServiceState::Stopped);
        assert_eq!(sp.failures_of("site", "web"), 0);
        assert_eq!(spawns.load(Ordering::SeqCst), before, "stop tay không được hồi sinh");
    }

    #[test]
    fn manual_start_resets_the_failure_counter() {
        let (mut sp, _alive, _spawns) = supervised();
        sp.start("site", Path::new("/tmp"), "web", "true").unwrap();
        sp.tick_at(Instant::now());
        assert!(sp.failures_of("site", "web") > 0);

        sp.start("site", Path::new("/tmp"), "web", "true").unwrap();
        assert_eq!(sp.failures_of("site", "web"), 0);
    }
}
