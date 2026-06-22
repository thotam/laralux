use crate::paths::LaragonPaths;
use crate::service::{Service, ServiceError, ServiceKind, SpawnSpec};
use std::path::PathBuf;

pub struct PhpFpmService {
    version: String,
}

impl PhpFpmService {
    pub fn new(version: impl Into<String>) -> Self {
        Self { version: version.into() }
    }
    pub fn socket_path(&self, paths: &LaragonPaths) -> PathBuf {
        paths.tmp().join("php-fpm.sock")
    }
    fn conf_path(&self, paths: &LaragonPaths) -> PathBuf {
        paths.etc_for("php").join(&self.version).join("php-fpm.conf")
    }
}

impl Service for PhpFpmService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::PhpFpm
    }
    fn name(&self) -> &str {
        "php-fpm"
    }
    fn write_config(&self, paths: &LaragonPaths) -> Result<(), ServiceError> {
        std::fs::create_dir_all(self.conf_path(paths).parent().unwrap())?;
        std::fs::create_dir_all(paths.tmp())?;
        let conf = format!(
            "[global]\n\
             pid = {pid}\n\
             error_log = {log}\n\
             daemonize = no\n\
             \n\
             [www]\n\
             listen = {sock}\n\
             listen.mode = 0660\n\
             pm = dynamic\n\
             pm.max_children = 10\n\
             pm.start_servers = 2\n\
             pm.min_spare_servers = 1\n\
             pm.max_spare_servers = 4\n",
            pid = paths.tmp().join("php-fpm.pid").display(),
            log = paths.log().join("php-fpm.log").display(),
            sock = self.socket_path(paths).display(),
        );
        std::fs::write(self.conf_path(paths), conf)?;
        Ok(())
    }
    fn command(&self, paths: &LaragonPaths) -> SpawnSpec {
        SpawnSpec::new(format!("php-fpm{}", self.version))
            .arg("-F") // foreground, so the orchestrator owns the process
            .arg("-y")
            .arg(self.conf_path(paths).display().to_string())
    }
    fn health_check(&self, paths: &LaragonPaths) -> Result<(), ServiceError> {
        if self.socket_path(paths).exists() {
            Ok(())
        } else {
            Err(ServiceError::HealthCheck("php-fpm socket missing".into()))
        }
    }
    fn pre_start(&self, paths: &LaragonPaths) -> Result<(), ServiceError> {
        // Clear a stale socket / orphaned master from a previous run so php-fpm
        // doesn't error with "Another FPM instance seems to already listen".
        crate::service::cleanup_stale_endpoint(
            Some(&paths.tmp().join("php-fpm.pid")),
            Some(&self.socket_path(paths)),
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
    fn command_uses_versioned_binary_and_foreground() {
        let p = LaragonPaths::new("/tmp/lara".into());
        let svc = PhpFpmService::new("8.4");
        let spec = svc.command(&p);
        assert_eq!(spec.program, "php-fpm8.4");
        assert!(spec.args.contains(&"-F".to_string()));
        assert!(spec.args.iter().any(|a| a.ends_with("php-fpm.conf")));
        assert_eq!(svc.kind(), ServiceKind::PhpFpm);
    }

    #[test]
    fn socket_path_is_under_tmp() {
        let p = LaragonPaths::new("/tmp/lara".into());
        let svc = PhpFpmService::new("8.4");
        assert_eq!(svc.socket_path(&p), std::path::Path::new("/tmp/lara/tmp/php-fpm.sock"));
    }

    #[test]
    fn write_config_defines_pool_with_socket() {
        let tmp = std::env::temp_dir().join(format!("lara-php-{}", std::process::id()));
        let p = LaragonPaths::new(tmp.clone());
        let svc = PhpFpmService::new("8.4");
        svc.write_config(&p).unwrap();
        let conf =
            std::fs::read_to_string(p.etc_for("php").join("8.4").join("php-fpm.conf")).unwrap();
        assert!(conf.contains("[www]"));
        assert!(conf.contains("listen = "));
        assert!(conf.contains("php-fpm.sock"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn write_config_omits_user_directive() {
        let tmp = std::env::temp_dir().join(format!("lara-php-nouser-{}", std::process::id()));
        let p = LaragonPaths::new(tmp.clone());
        let svc = PhpFpmService::new("8.4");
        svc.write_config(&p).unwrap();
        let conf =
            std::fs::read_to_string(p.etc_for("php").join("8.4").join("php-fpm.conf")).unwrap();
        assert!(!conf.contains("user ="), "pool must not set a user directive");
        // sanity: the pool is still defined and listens on the socket
        assert!(conf.contains("[www]"));
        assert!(conf.contains("listen = "));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn pre_start_removes_stale_socket() {
        let tmp = std::env::temp_dir().join(format!("lara-php-prestart-{}", std::process::id()));
        let p = LaragonPaths::new(tmp.clone());
        std::fs::create_dir_all(p.tmp()).unwrap();
        let sock = p.tmp().join("php-fpm.sock");
        std::fs::write(&sock, b"stale").unwrap();
        assert!(sock.exists());

        let svc = PhpFpmService::new("8.4");
        svc.pre_start(&p).unwrap(); // no pid file present -> just unlinks the socket
        assert!(!sock.exists(), "stale socket should be removed before start");
        std::fs::remove_dir_all(&tmp).ok();
    }
}
