use crate::paths::LaraluxPaths;
use crate::service::{probe_tcp, Service, ServiceError, ServiceKind, SpawnSpec};

pub struct RedisService {
    port: u16,
}

impl RedisService {
    pub fn new() -> Self {
        Self { port: 6379 }
    }
    fn conf_path(&self, paths: &LaraluxPaths) -> std::path::PathBuf {
        paths.etc_for("redis").join("redis.conf")
    }
}

impl Default for RedisService {
    fn default() -> Self {
        Self::new()
    }
}

impl Service for RedisService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Redis
    }
    fn name(&self) -> &str {
        "redis"
    }
    fn write_config(&self, paths: &LaraluxPaths) -> Result<(), ServiceError> {
        std::fs::create_dir_all(paths.etc_for("redis"))?;
        std::fs::create_dir_all(paths.data().join("redis"))?;
        let conf = format!(
            "port {port}\n\
             bind 127.0.0.1\n\
             dir {dir}\n\
             dbfilename dump.rdb\n\
             logfile {log}\n",
            port = self.port,
            dir = paths.data().join("redis").display(),
            log = paths.log().join("redis.log").display(),
        );
        std::fs::write(self.conf_path(paths), conf)?;
        Ok(())
    }
    fn command(&self, paths: &LaraluxPaths) -> SpawnSpec {
        SpawnSpec::new("redis-server").arg(self.conf_path(paths).display().to_string())
    }
    fn health_check(&self, _paths: &LaraluxPaths) -> Result<(), ServiceError> {
        probe_tcp(self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaraluxPaths;
    use crate::service::{Service, ServiceKind};

    #[test]
    fn command_runs_redis_server_with_conf() {
        let p = LaraluxPaths::new("/tmp/lara".into());
        let svc = RedisService::new();
        let spec = svc.command(&p);
        assert_eq!(spec.program, "redis-server");
        assert!(spec.args.iter().any(|a| a.ends_with("etc/redis/redis.conf")));
        assert_eq!(svc.kind(), ServiceKind::Redis);
    }

    #[test]
    fn write_config_creates_conf_with_port_and_dir() {
        let tmp = std::env::temp_dir().join(format!("lara-redis-{}", std::process::id()));
        let p = LaraluxPaths::new(tmp.clone());
        let svc = RedisService::new();
        svc.write_config(&p).unwrap();
        let conf = std::fs::read_to_string(p.etc_for("redis").join("redis.conf")).unwrap();
        assert!(conf.contains("port 6379"));
        assert!(conf.contains("dir "));
        std::fs::remove_dir_all(&tmp).ok();
    }
}
