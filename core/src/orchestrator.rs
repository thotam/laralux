use crate::paths::LaragonPaths;
use crate::process::{Process, ProcessSpawner};
use crate::service::{Service, ServiceError, ServiceKind, ServiceState};
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// A serializable point-in-time view of one service.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ServiceStatus {
    pub kind: ServiceKind,
    pub state: ServiceState,
}

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
        // Idempotent: a service already Running with a live process is left
        // untouched, so a repeated Start All (e.g. the tray firing it again
        // while the UI's start is mid-flight) never spawns a duplicate that
        // fights for the same port and orphans the original process.
        if self.state(kind) == ServiceState::Running {
            if let Some(h) = self.handles.get_mut(&kind) {
                if h.is_alive() {
                    return Ok(());
                }
            }
        }

        let svc = self
            .find(kind)
            .ok_or_else(|| ServiceError::Config(format!("no such service: {kind:?}")))?;

        // Capture what we need from svc before borrowing self mutably
        let needs_init = svc.needs_init(&self.paths);

        self.states.insert(kind, ServiceState::Starting);

        // Perform fallible work; on error, revert state to Stopped before returning
        if let Err(e) = self.do_start(kind, needs_init) {
            self.states.insert(kind, ServiceState::Stopped);
            return Err(e);
        }

        self.states.insert(kind, ServiceState::Running);
        Ok(())
    }

    fn do_start(&mut self, kind: ServiceKind, needs_init: bool) -> Result<(), ServiceError> {
        if needs_init {
            // Re-find the service after releasing the immutable borrow
            if let Some(svc) = self.find(kind) {
                svc.init(&self.paths)?;
            }
        }

        if let Some(svc) = self.find(kind) {
            svc.write_config(&self.paths)?;
            svc.pre_start(&self.paths)?;
            let mut spec = svc.command(&self.paths);
            spec.program = crate::bin::resolve_or_name(&spec.program, &crate::layout::managed_bin_dirs(&self.paths));
            let handle = self.spawner.spawn(&spec)?;
            self.handles.insert(kind, handle);
        }
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

    /// Reload a running service's config in place (SIGHUP) — used after writing
    /// new vhosts so nginx picks them up WITHOUT a stop/start that would rebind
    /// :80/:443 and race the dying process. No-op if the service isn't running.
    pub fn reload(&mut self, kind: ServiceKind) -> Result<(), ServiceError> {
        if self.state(kind) == ServiceState::Running {
            if let Some(h) = self.handles.get_mut(&kind) {
                if h.is_alive() {
                    return h
                        .reload()
                        .map_err(|e| ServiceError::Config(format!("reload {kind:?}: {e}")));
                }
            }
        }
        Ok(())
    }

    /// Swap the active php-fpm version. Stops php-fpm if running, repoints
    /// `bin/php/current` at the new version, and restarts it iff it had been
    /// running. Returns whether php-fpm had been running. The socket is
    /// version-independent, so nginx/vhosts are unaffected.
    pub fn replace_php_version(&mut self, version: &str) -> Result<bool, ServiceError> {
        let was_running = self.state(ServiceKind::PhpFpm) == ServiceState::Running;
        if was_running {
            let _ = self.stop(ServiceKind::PhpFpm);
        }
        // Reap any orphan php-fpm (master + workers) under bin/php and ensure the
        // just-stopped master is fully dead before the new version binds the
        // socket. Other tools live outside bin/php, so they are never touched.
        let _ = crate::orphans::reap(&self.paths.bin().join("php"), &self.tracked_pids());
        let full = crate::layout::resolve_installed_version(&self.paths, "php", version)
            .unwrap_or_else(|| version.to_string());
        crate::layout::set_current(&self.paths, "php", &full)
            .map_err(|e| ServiceError::Config(format!("set php current: {e}")))?;
        if was_running {
            self.start(ServiceKind::PhpFpm)?;
        }
        Ok(was_running)
    }

    /// Swap the active CoreDNS bases. Stops and removes any existing CoreDNS service,
    /// then starts a new one with the given bases (port 5353). If bases is empty,
    /// stops and removes without restarting.
    pub fn set_coredns(&mut self, bases: Vec<String>) -> Result<(), ServiceError> {
        let was_running = self.state(ServiceKind::Coredns) == ServiceState::Running;
        if was_running {
            let _ = self.stop(ServiceKind::Coredns);
        }
        self.services.retain(|s| s.kind() != ServiceKind::Coredns);
        if bases.is_empty() {
            return Ok(());
        }
        self.services
            .push(Box::new(crate::service::coredns::CorednsService::new(bases, 5353)));
        self.start(ServiceKind::Coredns)
    }

    /// Mark any service whose process has died as `Crashed`.
    pub fn refresh(&mut self) {
        let mut dead = Vec::new();
        for (kind, handle) in &mut self.handles {
            if !handle.is_alive() {
                dead.push(*kind);
            }
        }
        for k in dead {
            self.handles.remove(&k);
            self.states.insert(k, ServiceState::Crashed);
        }
    }

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

    /// Tracked PIDs of every live handle — the set a reap must NOT kill.
    fn tracked_pids(&self) -> Vec<u32> {
        self.handles.values().map(|h| h.pid()).collect()
    }

    /// Kill any *managed* process (executable under `bin/`) that this
    /// orchestrator does not track — an orphan left running by a prior session
    /// that would otherwise hold a port/socket/datadir lock and crash the fresh
    /// service. Live (tracked) services are kept. Best-effort; returns reaped PIDs.
    pub fn reap_orphans(&mut self) -> Vec<u32> {
        let keep = self.tracked_pids();
        crate::orphans::reap(&self.paths.bin(), &keep)
    }

    pub fn start_all(&mut self) -> Result<(), ServiceError> {
        // Clear orphans from a prior session before spawning, so the fresh
        // stack does not collide with leftovers on ports/sockets/locks.
        let _ = self.reap_orphans();
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

    /// Snapshot of every registered service in dependency-start order.
    pub fn snapshot(&self) -> Vec<ServiceStatus> {
        self.start_order()
            .into_iter()
            .map(|kind| ServiceStatus { kind, state: self.state(kind) })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn orch(spawner: FakeSpawner) -> Orchestrator {
        let services: Vec<Box<dyn Service>> =
            vec![Box::new(Dummy { kind: ServiceKind::Redis, name: "redis-server" })];
        Orchestrator::new(LaragonPaths::new("/tmp/lara".into()), services, Box::new(spawner))
    }

    struct FailingSpawner;
    impl ProcessSpawner for FailingSpawner {
        fn spawn(&self, _spec: &SpawnSpec) -> Result<Box<dyn Process>, std::io::Error> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "spawn failed",
            ))
        }
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

    #[test]
    fn spawn_failure_reverts_state_to_stopped() {
        let services: Vec<Box<dyn Service>> =
            vec![Box::new(Dummy { kind: ServiceKind::Redis, name: "redis-server" })];
        let mut o = Orchestrator::new(
            LaragonPaths::new("/tmp/lara".into()),
            services,
            Box::new(FailingSpawner),
        );

        let result = o.start(ServiceKind::Redis);
        assert!(result.is_err());
        assert_eq!(o.state(ServiceKind::Redis), ServiceState::Stopped);
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

    #[test]
    fn snapshot_lists_services_with_states() {
        let spawner = crate::process::FakeSpawner::new();
        let services: Vec<Box<dyn Service>> =
            vec![Box::new(Dummy { kind: ServiceKind::Redis, name: "redis-server" })];
        let mut o = Orchestrator::new(
            LaragonPaths::new("/tmp/lara".into()),
            services,
            Box::new(spawner),
        );

        let before = o.snapshot();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].kind, ServiceKind::Redis);
        assert_eq!(before[0].state, ServiceState::Stopped);

        o.start(ServiceKind::Redis).unwrap();
        let after = o.snapshot();
        assert_eq!(after[0].state, ServiceState::Running);
    }

    #[test]
    fn start_resolves_program_against_bin_dir() {
        // A fake binary placed in <root>/bin/redis-server/current/ (versioned layout)
        // should be spawned by absolute path.
        let root = std::env::temp_dir().join(format!("lara-orch-res-{}", std::process::id()));
        let cur = root.join("bin").join("redis-server").join("current");
        std::fs::create_dir_all(&cur).unwrap();
        let exe = cur.join("redis-server");
        std::fs::write(&exe, "x").unwrap();

        let spawner = crate::process::FakeSpawner::new();
        let log = spawner.log();
        let services: Vec<Box<dyn Service>> =
            vec![Box::new(Dummy { kind: ServiceKind::Redis, name: "redis-server" })];
        let mut o = Orchestrator::new(LaragonPaths::new(root.clone()), services, Box::new(spawner));

        o.start(ServiceKind::Redis).unwrap();
        assert_eq!(log.lock().unwrap()[0].program, exe.display().to_string());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn service_status_serializes_to_variant_names() {
        let s = ServiceStatus { kind: ServiceKind::Nginx, state: ServiceState::Running };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"kind":"Nginx","state":"Running"}"#);
    }

    #[test]
    fn replace_php_version_restarts_when_running() {
        let tmp = std::env::temp_dir().join(format!("lara-orch-php-{}", std::process::id()));
        let paths = LaragonPaths::new(tmp.clone());
        // Seed bin/php/8.3/ and bin/php/8.4/ so set_current has a target dir
        std::fs::create_dir_all(paths.version_dir("php", "8.4")).unwrap();
        std::fs::create_dir_all(paths.version_dir("php", "8.3")).unwrap();
        crate::layout::set_current(&paths, "php", "8.4").unwrap();
        let spawner = crate::process::FakeSpawner::new();
        let log = spawner.log();
        let mut orch = Orchestrator::new(
            paths,
            vec![Box::new(crate::service::php_fpm::PhpFpmService::new("8.4"))],
            Box::new(spawner),
        );
        orch.start(ServiceKind::PhpFpm).unwrap();
        assert_eq!(orch.state(ServiceKind::PhpFpm), ServiceState::Running);

        let was_running = orch.replace_php_version("8.3").unwrap();
        assert!(was_running);
        assert_eq!(orch.state(ServiceKind::PhpFpm), ServiceState::Running);
        // the spawned program is now the constant "php-fpm" (resolved via bin/php/current)
        let progs: Vec<String> = log.lock().unwrap().iter().map(|s| s.program.clone()).collect();
        assert_eq!(progs.last().unwrap(), "php-fpm");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn set_coredns_runs_when_bases_present_and_stops_when_empty() {
        let tmp = std::env::temp_dir().join(format!("lara-cdns-{}", std::process::id()));
        let paths = LaragonPaths::new(tmp.clone());
        let spawner = crate::process::FakeSpawner::new();
        let log = spawner.log();
        let mut orch = Orchestrator::new(paths, vec![], Box::new(spawner));

        orch.set_coredns(vec!["demo.dev".to_string()]).unwrap();
        assert_eq!(orch.state(ServiceKind::Coredns), ServiceState::Running);
        assert_eq!(log.lock().unwrap().last().unwrap().program, "coredns");

        orch.set_coredns(vec![]).unwrap();
        assert_eq!(orch.state(ServiceKind::Coredns), ServiceState::Stopped);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn replace_php_version_does_not_start_when_stopped() {
        let tmp = std::env::temp_dir().join(format!("lara-orch-php2-{}", std::process::id()));
        let paths = LaragonPaths::new(tmp.clone());
        // Seed bin/php/8.3/ so set_current has a target dir
        std::fs::create_dir_all(paths.version_dir("php", "8.3")).unwrap();
        let spawner = crate::process::FakeSpawner::new();
        let log = spawner.log();
        let mut orch = Orchestrator::new(
            paths,
            vec![Box::new(crate::service::php_fpm::PhpFpmService::new("8.4"))],
            Box::new(spawner),
        );
        let was_running = orch.replace_php_version("8.3").unwrap();
        assert!(!was_running);
        assert_eq!(orch.state(ServiceKind::PhpFpm), ServiceState::Stopped);
        assert!(log.lock().unwrap().is_empty()); // nothing spawned
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn reload_is_ok_when_running_and_noop_when_stopped() {
        let paths = LaragonPaths::new(std::env::temp_dir().join(format!("lara-reload-{}", std::process::id())));
        let services: Vec<Box<dyn Service>> =
            vec![Box::new(Dummy { kind: ServiceKind::Nginx, name: "nginx" })];
        let mut orch = Orchestrator::new(paths, services, Box::new(FakeSpawner::new()));
        // Stopped: reload is a no-op that still succeeds.
        assert!(orch.reload(ServiceKind::Nginx).is_ok());
        orch.start(ServiceKind::Nginx).unwrap();
        // Running: reload succeeds and leaves the service running (no respawn).
        assert!(orch.reload(ServiceKind::Nginx).is_ok());
        assert_eq!(orch.state(ServiceKind::Nginx), ServiceState::Running);
    }

    #[test]
    fn start_is_idempotent_for_running_service() {
        let paths = LaragonPaths::new(std::env::temp_dir().join(format!("lara-idem-{}", std::process::id())));
        let spawner = FakeSpawner::new();
        let log = spawner.log();
        let services: Vec<Box<dyn Service>> =
            vec![Box::new(Dummy { kind: ServiceKind::Mailpit, name: "mailpit" })];
        let mut orch = Orchestrator::new(paths, services, Box::new(spawner));
        orch.start(ServiceKind::Mailpit).unwrap();
        orch.start(ServiceKind::Mailpit).unwrap();
        assert_eq!(log.lock().unwrap().len(), 1, "second start must not spawn a duplicate");
        assert_eq!(orch.state(ServiceKind::Mailpit), ServiceState::Running);
    }
}
