use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error)]
pub enum PrivError {
    #[error("privileged io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("privileged command failed: {0}")]
    Command(String),
}

/// Operations that require elevated privileges or external trust stores.
pub trait Privileged: Send + Sync {
    fn write_etc_hosts(&self, new_content: &str) -> Result<(), PrivError>;
    fn install_mkcert_ca(&self) -> Result<(), PrivError>;
    fn setcap_nginx(&self, nginx_bin: &Path) -> Result<(), PrivError>;
}

// ---------- Real: sudo / mkcert ----------

pub struct SudoPrivileged;

impl SudoPrivileged {
    pub fn hosts_cp_command(src: &Path) -> (String, Vec<String>) {
        (
            "sudo".to_string(),
            vec!["cp".to_string(), src.display().to_string(), "/etc/hosts".to_string()],
        )
    }
    pub fn setcap_command(bin: &Path) -> (String, Vec<String>) {
        (
            "sudo".to_string(),
            vec![
                "setcap".to_string(),
                "cap_net_bind_service=+ep".to_string(),
                bin.display().to_string(),
            ],
        )
    }

    fn run(prog: &str, args: &[String]) -> Result<(), PrivError> {
        let status = std::process::Command::new(prog)
            .args(args)
            .status()
            .map_err(|e| PrivError::Command(format!("spawn {prog}: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(PrivError::Command(format!("{prog} exited with failure")))
        }
    }
}

impl Privileged for SudoPrivileged {
    fn write_etc_hosts(&self, new_content: &str) -> Result<(), PrivError> {
        let tmp = std::env::temp_dir().join("laragon-hosts.new");
        std::fs::write(&tmp, new_content)?;
        let (prog, args) = Self::hosts_cp_command(&tmp);
        Self::run(&prog, &args)
    }
    fn install_mkcert_ca(&self) -> Result<(), PrivError> {
        Self::run("mkcert", &["-install".to_string()])
    }
    fn setcap_nginx(&self, nginx_bin: &Path) -> Result<(), PrivError> {
        let (prog, args) = Self::setcap_command(nginx_bin);
        Self::run(&prog, &args)
    }
}

// ---------- Fake (used by sync tests) ----------

#[derive(Clone, Default)]
pub struct FakePrivileged {
    hosts_writes: Arc<Mutex<Vec<String>>>,
    installed_ca: Arc<Mutex<bool>>,
    setcap_done: Arc<Mutex<bool>>,
}

impl FakePrivileged {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn hosts_writes(&self) -> Arc<Mutex<Vec<String>>> {
        self.hosts_writes.clone()
    }
    pub fn installed_ca(&self) -> bool {
        *self.installed_ca.lock().unwrap()
    }
}

impl Privileged for FakePrivileged {
    fn write_etc_hosts(&self, new_content: &str) -> Result<(), PrivError> {
        self.hosts_writes.lock().unwrap().push(new_content.to_string());
        Ok(())
    }
    fn install_mkcert_ca(&self) -> Result<(), PrivError> {
        *self.installed_ca.lock().unwrap() = true;
        Ok(())
    }
    fn setcap_nginx(&self, _nginx_bin: &Path) -> Result<(), PrivError> {
        *self.setcap_done.lock().unwrap() = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn sudo_command_builders_are_correct() {
        let (prog, args) = SudoPrivileged::hosts_cp_command(Path::new("/tmp/hosts.new"));
        assert_eq!(prog, "sudo");
        assert_eq!(args, vec!["cp".to_string(), "/tmp/hosts.new".to_string(), "/etc/hosts".to_string()]);

        let (prog2, args2) = SudoPrivileged::setcap_command(Path::new("/usr/sbin/nginx"));
        assert_eq!(prog2, "sudo");
        assert_eq!(
            args2,
            vec![
                "setcap".to_string(),
                "cap_net_bind_service=+ep".to_string(),
                "/usr/sbin/nginx".to_string(),
            ]
        );
    }

    #[test]
    fn fake_records_hosts_write() {
        let f = FakePrivileged::new();
        let log = f.hosts_writes();
        f.write_etc_hosts("# BEGIN laragon-linux\n# END laragon-linux\n").unwrap();
        assert_eq!(log.lock().unwrap().len(), 1);
        assert!(log.lock().unwrap()[0].contains("laragon-linux"));
    }
}
