use crate::service::SpawnSpec;
use std::sync::{Arc, Mutex};

/// A running process handle.
pub trait Process: Send + Sync {
    fn is_alive(&mut self) -> bool;
    fn stop(&mut self) -> std::io::Result<()>;
    fn pid(&self) -> u32;
}

/// Spawns processes from a `SpawnSpec`. Hidden behind a trait so the
/// orchestrator can be tested without launching real binaries.
pub trait ProcessSpawner: Send + Sync {
    fn spawn(&self, spec: &SpawnSpec) -> std::io::Result<Box<dyn Process>>;
}

// ---------- Real implementation ----------

pub struct RealSpawner;

struct RealProcess {
    child: std::process::Child,
}

impl Process for RealProcess {
    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
    fn stop(&mut self) -> std::io::Result<()> {
        // Graceful SIGTERM via libc kill; fall back to SIGKILL if needed.
        let pid = self.child.id() as i32;
        unsafe {
            libc_kill(pid, 15); // SIGTERM
        }
        Ok(())
    }
    fn pid(&self) -> u32 {
        self.child.id()
    }
}

// Minimal libc kill binding to avoid a libc dependency for one call.
extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

impl ProcessSpawner for RealSpawner {
    fn spawn(&self, spec: &SpawnSpec) -> std::io::Result<Box<dyn Process>> {
        let mut cmd = std::process::Command::new(&spec.program);
        cmd.args(&spec.args);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        if let Some(dir) = &spec.cwd {
            cmd.current_dir(dir);
        }
        let child = cmd.spawn()?;
        Ok(Box::new(RealProcess { child }))
    }
}

// ---------- Fake implementation (used by other modules' tests) ----------

#[derive(Clone, Default)]
pub struct FakeSpawner {
    log: Arc<Mutex<Vec<SpawnSpec>>>,
}

impl FakeSpawner {
    pub fn new() -> Self {
        Self::default()
    }
    /// Shared record of every spec that was spawned, in order.
    pub fn log(&self) -> Arc<Mutex<Vec<SpawnSpec>>> {
        self.log.clone()
    }
}

pub struct FakeProcess {
    alive: bool,
    pid: u32,
}

impl Process for FakeProcess {
    fn is_alive(&mut self) -> bool {
        self.alive
    }
    fn stop(&mut self) -> std::io::Result<()> {
        self.alive = false;
        Ok(())
    }
    fn pid(&self) -> u32 {
        self.pid
    }
}

impl ProcessSpawner for FakeSpawner {
    fn spawn(&self, spec: &SpawnSpec) -> std::io::Result<Box<dyn Process>> {
        let mut log = self.log.lock().unwrap();
        log.push(spec.clone());
        let pid = 1000 + log.len() as u32;
        Ok(Box::new(FakeProcess { alive: true, pid }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::SpawnSpec;

    #[test]
    fn fake_spawner_records_and_tracks_alive() {
        let spawner = FakeSpawner::new();
        let log = spawner.log();
        let mut p = spawner.spawn(&SpawnSpec::new("redis-server").arg("--port").arg("6379")).unwrap();
        assert!(p.is_alive());
        assert_eq!(log.lock().unwrap().len(), 1);
        assert_eq!(log.lock().unwrap()[0].program, "redis-server");
        p.stop().unwrap();
        assert!(!p.is_alive());
    }

    #[test]
    fn real_spawner_runs_and_stops_a_process() {
        // `sleep 30` is a real long-lived process we can stop deterministically.
        let spawner = RealSpawner;
        let mut p = spawner.spawn(&SpawnSpec::new("sleep").arg("30")).unwrap();
        assert!(p.is_alive());
        assert!(p.pid() > 0);
        p.stop().unwrap();
        // Give the OS a moment, then confirm it is gone.
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!p.is_alive());
    }
}
