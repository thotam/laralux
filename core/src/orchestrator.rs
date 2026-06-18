use crate::paths::LaragonPaths;
use crate::process::{Process, ProcessSpawner};
use crate::service::{Service, ServiceError, ServiceKind, ServiceState};
use std::collections::HashMap;

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

        self.states.insert(kind, ServiceState::Running);
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

    fn orch(spawner: FakeSpawner) -> Orchestrator {
        let services: Vec<Box<dyn Service>> =
            vec![Box::new(Dummy { kind: ServiceKind::Redis, name: "redis-server" })];
        Orchestrator::new(LaragonPaths::new("/tmp/lara".into()), services, Box::new(spawner))
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
}
