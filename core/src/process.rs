use crate::service::SpawnSpec;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A running process handle.
pub trait Process: Send + Sync {
    fn is_alive(&mut self) -> bool;
    fn stop(&mut self) -> std::io::Result<()>;
    /// Ask the process to reload its configuration in place (SIGHUP). For nginx
    /// this re-reads vhosts WITHOUT closing the listening sockets, so applying a
    /// new site never rebinds :80/:443 (no "Address in use", zero downtime).
    fn reload(&mut self) -> std::io::Result<()>;
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
        // Graceful shutdown then guaranteed termination: SIGTERM, wait up to ~3s
        // for the process to actually exit (and reap it), else SIGKILL and reap.
        // Blocking until the process is gone means a subsequent start() rebinds a
        // freed port instead of racing the dying process ("Address in use").
        // Signal the whole process GROUP (negative pid), not just the leader, so
        // any children the process forked die with it instead of orphaning. The
        // leader is spawned as its own group leader (pgid == pid), so `-pid`
        // targets it and every descendant.
        let pid = self.child.id() as i32;
        unsafe {
            libc_kill(-pid, 15); // SIGTERM to the group
        }
        let deadline = Instant::now() + Duration::from_millis(3000);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => {
                    // Leader is gone; make sure no child outlived it.
                    unsafe { libc_kill(-pid, 9); }
                    return Ok(());
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
                Err(_) => break,
            }
        }
        unsafe {
            libc_kill(-pid, 9); // SIGKILL the group
        }
        let _ = self.child.wait(); // reap the leader so it can't linger as a zombie
        Ok(())
    }
    fn reload(&mut self) -> std::io::Result<()> {
        let pid = self.child.id() as i32;
        unsafe {
            libc_kill(pid, 1); // SIGHUP
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
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new(&spec.program);
        cmd.args(&spec.args);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        if let Some(dir) = &spec.cwd {
            cmd.current_dir(dir);
        }
        // Put the child in its own process group (pgid == its pid). All the
        // descendants it forks — `npm run dev` -> node -> esbuild, `artisan
        // serve` -> php router, a queue worker's job subprocesses — stay in that
        // group (job control is off in the non-interactive shells we spawn), so
        // stop() can signal the whole tree and never leave orphans behind.
        cmd.process_group(0);
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
    fn reload(&mut self) -> std::io::Result<()> {
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

    /// Returns true while `pid` still exists (kill(pid, 0) succeeds).
    fn pid_alive(pid: i32) -> bool {
        unsafe { libc_kill(pid, 0) == 0 }
    }

    #[test]
    fn stop_kills_child_processes_not_just_the_leader() {
        // Mirror how SiteProcs spawns: `sh -c "exec <cmd>"`. Here <cmd> is a shell
        // that backgrounds a long-lived child, records the child PID, then waits —
        // so the tracked leader has a descendant that must also die on stop().
        // In a non-interactive shell job control is off, so the backgrounded child
        // stays in the leader's process group (the whole point of the fix).
        let pidfile = std::env::temp_dir().join(format!("lara-proc-child-{}", std::process::id()));
        let _ = std::fs::remove_file(&pidfile);
        let script = format!("exec sh -c 'sleep 60 & echo $! > {}; wait'", pidfile.display());
        let spawner = RealSpawner;
        let mut p = spawner.spawn(&SpawnSpec::new("sh").arg("-c").arg(script)).unwrap();

        // Wait for the child to record its PID.
        let mut child_pid = 0i32;
        for _ in 0..100 {
            if let Ok(s) = std::fs::read_to_string(&pidfile) {
                if let Ok(n) = s.trim().parse::<i32>() {
                    child_pid = n;
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(child_pid > 0, "child never recorded its PID");
        assert!(pid_alive(child_pid), "child should be running before stop");

        p.stop().unwrap();

        // The orphaned child must be dead too, not reparented-and-running.
        let mut gone = false;
        for _ in 0..100 {
            if !pid_alive(child_pid) {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // Clean up the child if the fix is missing, so the test host isn't littered.
        if !gone {
            unsafe { libc_kill(child_pid, 9); }
        }
        let _ = std::fs::remove_file(&pidfile);
        assert!(gone, "child process {child_pid} survived stop() — stop killed only the leader, not the group");
    }
}
