use crate::paths::LaraluxPaths;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum ServiceKind {
    Nginx,
    PhpFpm,
    Mariadb,
    Postgres,
    Redis,
    Mailpit,
    Coredns,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ServiceState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config error: {0}")]
    Config(String),
    #[error("health check failed: {0}")]
    HealthCheck(String),
    #[error("init failed: {0}")]
    Init(String),
}

/// A fully-specified command to spawn a service process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
}

impl SpawnSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self { program: program.into(), args: Vec::new(), env: Vec::new(), cwd: None }
    }
    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }
    pub fn args<I, S>(mut self, items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(items.into_iter().map(Into::into));
        self
    }
    pub fn env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }
    pub fn cwd(mut self, dir: PathBuf) -> Self {
        self.cwd = Some(dir);
        self
    }
}

/// A managed service (nginx, php-fpm, mariadb, redis, mailpit).
pub trait Service: Send + Sync {
    fn kind(&self) -> ServiceKind;
    fn name(&self) -> &str;
    fn deps(&self) -> &[ServiceKind] {
        &[]
    }
    fn write_config(&self, _paths: &LaraluxPaths) -> Result<(), ServiceError> {
        Ok(())
    }
    fn command(&self, paths: &LaraluxPaths) -> SpawnSpec;
    fn health_check(&self, paths: &LaraluxPaths) -> Result<(), ServiceError>;
    fn needs_init(&self, _paths: &LaraluxPaths) -> bool {
        false
    }
    fn init(&self, _paths: &LaraluxPaths) -> Result<(), ServiceError> {
        Ok(())
    }
    /// Cleanup run immediately before spawning (after `write_config`).
    /// Used to clear a stale unix socket / orphaned process left by a previous
    /// run so the daemon can bind cleanly. Default: no-op.
    fn pre_start(&self, _paths: &LaraluxPaths) -> Result<(), ServiceError> {
        Ok(())
    }
}

/// Best-effort cleanup of a stale daemon endpoint left by a previous run:
/// SIGTERM the PID in `pid_file` (if any), then unlink a leftover `socket`.
/// Both steps are best-effort and never error (a missing file is fine).
pub fn cleanup_stale_endpoint(pid_file: Option<&std::path::Path>, socket: Option<&std::path::Path>) {
    if let Some(pf) = pid_file {
        if let Ok(text) = std::fs::read_to_string(pf) {
            if let Ok(pid) = text.trim().parse::<i32>() {
                if pid > 1 {
                    let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
                }
            }
        }
    }
    if let Some(sock) = socket {
        if sock.exists() {
            let _ = std::fs::remove_file(sock);
        }
    }
}

/// Returns Ok if a TCP connect to `127.0.0.1:port` succeeds within 1s.
pub fn probe_tcp(port: u16) -> Result<(), ServiceError> {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;
    let addr = ("127.0.0.1", port)
        .to_socket_addrs()
        .map_err(ServiceError::Io)?
        .next()
        .ok_or_else(|| ServiceError::HealthCheck("no address".into()))?;
    TcpStream::connect_timeout(&addr, Duration::from_secs(1))
        .map(|_| ())
        .map_err(|e| ServiceError::HealthCheck(format!("port {port}: {e}")))
}

pub mod coredns;
pub mod mailpit;
pub mod mariadb;
pub mod nginx;
pub mod php_fpm;
pub mod postgres;
pub mod redis;
pub mod registry;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaraluxPaths;

    struct Fake;
    impl Service for Fake {
        fn kind(&self) -> ServiceKind {
            ServiceKind::Redis
        }
        fn name(&self) -> &str {
            "fake"
        }
        fn command(&self, _paths: &LaraluxPaths) -> SpawnSpec {
            SpawnSpec::new("true")
        }
        fn health_check(&self, _paths: &LaraluxPaths) -> Result<(), ServiceError> {
            Ok(())
        }
    }

    #[test]
    fn trait_defaults_work() {
        let f = Fake;
        let p = LaraluxPaths::new("/tmp/x".into());
        assert_eq!(f.name(), "fake");
        assert_eq!(f.kind(), ServiceKind::Redis);
        assert!(f.deps().is_empty());
        assert!(!f.needs_init(&p));
        assert_eq!(f.command(&p).program, "true");
    }

    #[test]
    fn spawnspec_builder_sets_fields() {
        let s = SpawnSpec::new("nginx").arg("-t").env("FOO", "bar");
        assert_eq!(s.program, "nginx");
        assert_eq!(s.args, vec!["-t".to_string()]);
        assert_eq!(s.env, vec![("FOO".to_string(), "bar".to_string())]);
    }
}
