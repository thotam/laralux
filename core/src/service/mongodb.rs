use crate::paths::LaraluxPaths;
use crate::service::{probe_tcp, Service, ServiceError, ServiceKind, SpawnSpec};
use std::path::PathBuf;

pub struct MongodbService {
    port: u16,
}

impl MongodbService {
    pub fn new() -> Self {
        Self { port: 27017 }
    }
    fn dbpath(&self, paths: &LaraluxPaths) -> PathBuf {
        paths.data().join("mongodb")
    }
}

impl Default for MongodbService {
    fn default() -> Self {
        Self::new()
    }
}

impl Service for MongodbService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Mongodb
    }
    fn name(&self) -> &str {
        "mongodb"
    }
    fn write_config(&self, paths: &LaraluxPaths) -> Result<(), ServiceError> {
        // mongod requires the dbpath dir to pre-exist; it populates the
        // WiredTiger files itself on first start (no separate init step).
        std::fs::create_dir_all(self.dbpath(paths))?;
        std::fs::create_dir_all(paths.log())?;
        std::fs::create_dir_all(paths.tmp())?;
        Ok(())
    }
    fn command(&self, paths: &LaraluxPaths) -> SpawnSpec {
        SpawnSpec::new("mongod")
            .arg("--dbpath")
            .arg(self.dbpath(paths).display().to_string())
            .arg("--port")
            .arg(self.port.to_string())
            .arg("--bind_ip")
            .arg("127.0.0.1")
            .arg("--unixSocketPrefix")
            .arg(paths.tmp().display().to_string())
            .arg("--logpath")
            .arg(paths.log().join("mongodb.log").display().to_string())
            .arg("--logappend")
    }
    fn health_check(&self, _paths: &LaraluxPaths) -> Result<(), ServiceError> {
        probe_tcp(self.port)
    }
    fn pre_start(&self, paths: &LaraluxPaths) -> Result<(), ServiceError> {
        // Clear a stale lock + unix socket from a previous run.
        crate::service::cleanup_stale_endpoint(
            None,
            Some(&paths.tmp().join("mongodb-27017.sock")),
        );
        let _ = std::fs::remove_file(self.dbpath(paths).join("mongod.lock"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Service, ServiceKind};

    #[test]
    fn command_and_kind() {
        let p = LaraluxPaths::new("/tmp/lara".into());
        let svc = MongodbService::new();
        let spec = svc.command(&p);
        assert_eq!(spec.program, "mongod");
        assert!(spec.args.iter().any(|a| a == "--dbpath"));
        assert!(spec.args.iter().any(|a| a == "27017"));
        assert!(spec.args.iter().any(|a| a == "127.0.0.1"));
        assert!(spec.args.iter().any(|a| a.ends_with("mongodb.log")));
        assert!(spec.args.iter().any(|a| a == "--unixSocketPrefix"));
        assert_eq!(svc.kind(), ServiceKind::Mongodb);
        assert_eq!(svc.name(), "mongodb");
    }
}
