use crate::coredns::corefile;
use crate::paths::LaragonPaths;
use crate::service::{probe_tcp, Service, ServiceError, ServiceKind, SpawnSpec};

pub struct CorednsService {
    bases: Vec<String>,
    port: u16,
}

impl CorednsService {
    pub fn new(bases: Vec<String>, port: u16) -> Self {
        Self { bases, port }
    }
    fn conf_path(&self, paths: &LaragonPaths) -> std::path::PathBuf {
        paths.etc_for("coredns").join("Corefile")
    }
}

impl Service for CorednsService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Coredns
    }
    fn name(&self) -> &str {
        "coredns"
    }
    fn write_config(&self, paths: &LaragonPaths) -> Result<(), ServiceError> {
        std::fs::create_dir_all(paths.etc_for("coredns"))?;
        std::fs::write(self.conf_path(paths), corefile(&self.bases, self.port))?;
        Ok(())
    }
    fn command(&self, paths: &LaragonPaths) -> SpawnSpec {
        SpawnSpec::new("coredns")
            .arg("-conf")
            .arg(self.conf_path(paths).display().to_string())
    }
    fn health_check(&self, _paths: &LaragonPaths) -> Result<(), ServiceError> {
        probe_tcp(self.port)
    }
}
