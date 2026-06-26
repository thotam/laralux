use crate::paths::LaragonPaths;
use crate::service::{probe_tcp, Service, ServiceError, ServiceKind, SpawnSpec};
use std::path::PathBuf;

pub struct MariadbService {
    port: u16,
}

impl MariadbService {
    pub fn new() -> Self {
        Self { port: 3306 }
    }
    fn cnf_path(&self, paths: &LaragonPaths) -> PathBuf {
        paths.etc_for("mariadb").join("my.cnf")
    }
    fn datadir(&self, paths: &LaragonPaths) -> PathBuf {
        paths.data().join("mariadb")
    }
    fn basedir(&self, paths: &LaragonPaths) -> PathBuf {
        paths.bin().join("mariadb").join("current")
    }
    fn install_db_args(&self, paths: &LaragonPaths) -> Vec<String> {
        vec![
            "--no-defaults".to_string(),
            format!("--basedir={}", self.basedir(paths).display()),
            format!("--datadir={}", self.datadir(paths).display()),
            "--auth-root-authentication-method=normal".to_string(),
        ]
    }
}

impl Default for MariadbService {
    fn default() -> Self {
        Self::new()
    }
}

impl Service for MariadbService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Mariadb
    }
    fn name(&self) -> &str {
        "mariadb"
    }
    fn write_config(&self, paths: &LaragonPaths) -> Result<(), ServiceError> {
        std::fs::create_dir_all(paths.etc_for("mariadb"))?;
        std::fs::create_dir_all(self.datadir(paths))?;
        let conf = format!(
            "[mysqld]\n\
             datadir={datadir}\n\
             socket={sock}\n\
             port={port}\n\
             bind-address=127.0.0.1\n\
             pid-file={pid}\n\
             log-error={log}\n",
            datadir = self.datadir(paths).display(),
            sock = paths.tmp().join("mysql.sock").display(),
            port = self.port,
            pid = paths.tmp().join("mariadb.pid").display(),
            log = paths.log().join("mariadb.log").display(),
        );
        std::fs::write(self.cnf_path(paths), conf)?;
        Ok(())
    }
    fn needs_init(&self, paths: &LaragonPaths) -> bool {
        !self.datadir(paths).join("mysql").is_dir()
    }
    fn init(&self, paths: &LaragonPaths) -> Result<(), ServiceError> {
        self.write_config(paths)?;
        let tool = crate::bin::resolve_bin("mariadb-install-db", &crate::layout::managed_bin_dirs(paths))
            .ok_or_else(|| ServiceError::Init("mariadb-install-db not found".into()))?;
        let status = std::process::Command::new(&tool)
            .args(self.install_db_args(paths))
            .status()
            .map_err(|e| ServiceError::Init(format!("mariadb-install-db: {e}")))?;
        if !status.success() {
            return Err(ServiceError::Init("mariadb-install-db failed".into()));
        }
        Ok(())
    }
    fn command(&self, paths: &LaragonPaths) -> SpawnSpec {
        SpawnSpec::new("mariadbd")
            .arg(format!("--defaults-file={}", self.cnf_path(paths).display()))
            .arg(format!("--basedir={}", self.basedir(paths).display()))
    }
    fn health_check(&self, _paths: &LaragonPaths) -> Result<(), ServiceError> {
        probe_tcp(self.port)
    }
    fn pre_start(&self, paths: &LaragonPaths) -> Result<(), ServiceError> {
        // Clear a stale unix socket / orphaned mariadbd from a previous run.
        crate::service::cleanup_stale_endpoint(
            Some(&paths.tmp().join("mariadb.pid")),
            Some(&paths.tmp().join("mysql.sock")),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaragonPaths;
    use crate::service::{Service, ServiceKind};

    #[test]
    fn command_and_kind() {
        let p = LaragonPaths::new("/tmp/lara".into());
        let svc = MariadbService::new();
        let spec = svc.command(&p);
        assert_eq!(spec.program, "mariadbd");
        assert!(spec.args.iter().any(|a| a.contains("--defaults-file=")));
        assert!(spec.args.iter().any(|a| a.starts_with("--basedir=")));
        assert_eq!(svc.kind(), ServiceKind::Mariadb);
    }

    #[test]
    fn needs_init_true_when_datadir_empty() {
        let tmp = std::env::temp_dir().join(format!("lara-maria-{}", std::process::id()));
        let p = LaragonPaths::new(tmp.clone());
        let svc = MariadbService::new();
        assert!(svc.needs_init(&p));
        std::fs::create_dir_all(p.data().join("mariadb").join("mysql")).unwrap();
        assert!(!svc.needs_init(&p));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn write_config_sets_datadir_and_port() {
        let tmp = std::env::temp_dir().join(format!("lara-mariacfg-{}", std::process::id()));
        let p = LaragonPaths::new(tmp.clone());
        let svc = MariadbService::new();
        svc.write_config(&p).unwrap();
        let conf = std::fs::read_to_string(p.etc_for("mariadb").join("my.cnf")).unwrap();
        assert!(conf.contains("datadir"));
        assert!(conf.contains("port=3306") || conf.contains("port = 3306"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn install_db_args_use_no_defaults_not_defaults_file() {
        let p = LaragonPaths::new("/tmp/lara".into());
        let svc = MariadbService::new();
        let args = svc.install_db_args(&p);
        assert!(args.contains(&"--no-defaults".to_string()));
        assert!(args.iter().any(|a| a.starts_with("--datadir=")));
        assert!(args.iter().any(|a| a.contains("auth-root-authentication-method=normal")));
        assert!(!args.iter().any(|a| a.contains("--defaults-file")));
        assert!(args.iter().any(|a| a.starts_with("--basedir=")));
    }
}
