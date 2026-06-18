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
            let spec = svc.command(&self.paths);
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
    fn service_status_serializes_to_variant_names() {
        let s = ServiceStatus { kind: ServiceKind::Nginx, state: ServiceState::Running };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"kind":"Nginx","state":"Running"}"#);
    }
}
