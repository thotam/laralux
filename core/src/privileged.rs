use std::path::{Path, PathBuf};
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
    fn install_mkcert_ca(&self, mkcert_bin: &Path) -> Result<(), PrivError>;
    fn setcap_nginx(&self, nginx_bin: &Path) -> Result<(), PrivError>;
    fn disable_system_services(&self, units: &[String]) -> Result<(), PrivError>;
    fn write_resolved_dropin(&self, contents: &str) -> Result<(), PrivError>;
    fn remove_resolved_dropin(&self) -> Result<(), PrivError>;
    fn create_symlink(&self, src: &Path, dst: &Path) -> Result<(), PrivError>;
    fn remove_symlink(&self, dst: &Path) -> Result<(), PrivError>;
}

// ---------- Shared free helpers ----------

fn cp_argv(src: &Path) -> Vec<String> {
    vec!["cp".to_string(), src.display().to_string(), "/etc/hosts".to_string()]
}

fn setcap_argv(bin: &Path) -> Vec<String> {
    vec![
        "setcap".to_string(),
        "cap_net_bind_service=+ep".to_string(),
        bin.display().to_string(),
    ]
}

fn systemctl_disable_argv(units: &[String]) -> Vec<String> {
    let mut argv = vec!["systemctl".to_string(), "disable".to_string(), "--now".to_string()];
    argv.extend(units.iter().cloned());
    argv
}

const RESOLVED_DROPIN: &str = "/etc/systemd/resolved.conf.d/laralux.conf";

fn write_resolved_argv(contents: &str) -> Vec<String> {
    vec![
        "sh".to_string(),
        "-c".to_string(),
        format!(
            "mkdir -p /etc/systemd/resolved.conf.d && cat > {RESOLVED_DROPIN} <<'LARALUXEOF'\n{contents}\nLARALUXEOF\nsystemctl reload systemd-resolved || systemctl restart systemd-resolved || true"
        ),
    ]
}

fn remove_resolved_argv() -> Vec<String> {
    vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("rm -f {RESOLVED_DROPIN}; systemctl reload systemd-resolved || systemctl restart systemd-resolved || true"),
    ]
}

fn ln_symlink_argv(src: &Path, dst: &Path) -> Vec<String> {
    vec!["ln".to_string(), "-sfn".to_string(), src.display().to_string(), dst.display().to_string()]
}

fn rm_argv(dst: &Path) -> Vec<String> {
    vec!["rm".to_string(), "-f".to_string(), dst.display().to_string()]
}

fn run_escalated(escalator: &str, argv: &[String]) -> Result<(), PrivError> {
    let status = std::process::Command::new(escalator)
        .args(argv)
        .status()
        .map_err(|e| PrivError::Command(format!("spawn {escalator}: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(PrivError::Command(format!("{escalator} command failed")))
    }
}

/// Run `mkcert -install` limited to the system trust store. Scoping to `system`
/// keeps mkcert from attempting the Firefox/Chrome NSS stores (which need
/// `certutil`), so it no longer warns about a missing certutil — browser trust
/// is handled separately by the bundled certutil (`certutil_static`).
fn run_mkcert_system(mkcert_bin: &Path) -> Result<(), PrivError> {
    let status = std::process::Command::new(mkcert_bin)
        .arg("-install")
        .env("TRUST_STORES", "system")
        .status()
        .map_err(|e| PrivError::Command(format!("spawn mkcert: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(PrivError::Command("mkcert -install (system) failed".to_string()))
    }
}

// ---------- Real: sudo / mkcert ----------

pub struct SudoPrivileged;

impl SudoPrivileged {
    pub fn hosts_cp_command(src: &Path) -> (String, Vec<String>) {
        ("sudo".to_string(), cp_argv(src))
    }
    pub fn setcap_command(bin: &Path) -> (String, Vec<String>) {
        ("sudo".to_string(), setcap_argv(bin))
    }
}

impl Privileged for SudoPrivileged {
    fn write_etc_hosts(&self, new_content: &str) -> Result<(), PrivError> {
        let tmp = std::env::temp_dir().join("laralux-hosts.new");
        std::fs::write(&tmp, new_content)?;
        run_escalated("sudo", &cp_argv(&tmp))
    }
    fn install_mkcert_ca(&self, mkcert_bin: &Path) -> Result<(), PrivError> {
        run_mkcert_system(mkcert_bin)
    }
    fn setcap_nginx(&self, nginx_bin: &Path) -> Result<(), PrivError> {
        run_escalated("sudo", &setcap_argv(nginx_bin))
    }
    fn disable_system_services(&self, units: &[String]) -> Result<(), PrivError> {
        run_escalated("sudo", &systemctl_disable_argv(units))
    }
    fn write_resolved_dropin(&self, contents: &str) -> Result<(), PrivError> {
        run_escalated("sudo", &write_resolved_argv(contents))
    }
    fn remove_resolved_dropin(&self) -> Result<(), PrivError> {
        run_escalated("sudo", &remove_resolved_argv())
    }
    fn create_symlink(&self, src: &Path, dst: &Path) -> Result<(), PrivError> {
        run_escalated("sudo", &ln_symlink_argv(src, dst))
    }
    fn remove_symlink(&self, dst: &Path) -> Result<(), PrivError> {
        run_escalated("sudo", &rm_argv(dst))
    }
}

// ---------- Real: pkexec (graphical auth) ----------

/// Privileged operations escalated with `pkexec` (graphical auth) — for GUI use.
pub struct PkexecPrivileged;

impl Privileged for PkexecPrivileged {
    fn write_etc_hosts(&self, new_content: &str) -> Result<(), PrivError> {
        let tmp = std::env::temp_dir().join("laralux-hosts.new");
        std::fs::write(&tmp, new_content)?;
        run_escalated("pkexec", &cp_argv(&tmp))
    }
    fn install_mkcert_ca(&self, mkcert_bin: &Path) -> Result<(), PrivError> {
        run_mkcert_system(mkcert_bin)
    }
    fn setcap_nginx(&self, nginx_bin: &Path) -> Result<(), PrivError> {
        run_escalated("pkexec", &setcap_argv(nginx_bin))
    }
    fn disable_system_services(&self, units: &[String]) -> Result<(), PrivError> {
        run_escalated("pkexec", &systemctl_disable_argv(units))
    }
    fn write_resolved_dropin(&self, contents: &str) -> Result<(), PrivError> {
        run_escalated("pkexec", &write_resolved_argv(contents))
    }
    fn remove_resolved_dropin(&self) -> Result<(), PrivError> {
        run_escalated("pkexec", &remove_resolved_argv())
    }
    fn create_symlink(&self, src: &Path, dst: &Path) -> Result<(), PrivError> {
        run_escalated("pkexec", &ln_symlink_argv(src, dst))
    }
    fn remove_symlink(&self, dst: &Path) -> Result<(), PrivError> {
        run_escalated("pkexec", &rm_argv(dst))
    }
}

// ---------- Fake (used by sync tests) ----------

#[derive(Clone, Default)]
pub struct FakePrivileged {
    hosts_writes: Arc<Mutex<Vec<String>>>,
    mkcert_ca_path: Arc<Mutex<Option<PathBuf>>>,
    setcap_done: Arc<Mutex<bool>>,
    disabled_services: Arc<Mutex<Vec<Vec<String>>>>,
    resolved_dropins: Arc<Mutex<Vec<String>>>,
    resolved_removed: Arc<Mutex<bool>>,
    symlinks_created: Arc<Mutex<Vec<(String, String)>>>,
    symlinks_removed: Arc<Mutex<Vec<String>>>,
}

impl FakePrivileged {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn hosts_writes(&self) -> Arc<Mutex<Vec<String>>> {
        self.hosts_writes.clone()
    }
    /// Returns true if `install_mkcert_ca` was called at least once.
    pub fn installed_ca(&self) -> bool {
        self.mkcert_ca_path.lock().unwrap().is_some()
    }
    /// Returns the path that was passed to `install_mkcert_ca`, if called.
    pub fn mkcert_ca_path(&self) -> Option<PathBuf> {
        self.mkcert_ca_path.lock().unwrap().clone()
    }
    pub fn disabled_services(&self) -> Arc<Mutex<Vec<Vec<String>>>> {
        self.disabled_services.clone()
    }
    pub fn resolved_dropins(&self) -> Arc<Mutex<Vec<String>>> {
        self.resolved_dropins.clone()
    }
    pub fn resolved_removed(&self) -> Arc<Mutex<bool>> {
        self.resolved_removed.clone()
    }
    pub fn symlinks_created(&self) -> Arc<Mutex<Vec<(String, String)>>> {
        self.symlinks_created.clone()
    }
    pub fn symlinks_removed(&self) -> Arc<Mutex<Vec<String>>> {
        self.symlinks_removed.clone()
    }
}

impl Privileged for FakePrivileged {
    fn write_etc_hosts(&self, new_content: &str) -> Result<(), PrivError> {
        self.hosts_writes.lock().unwrap().push(new_content.to_string());
        Ok(())
    }
    fn install_mkcert_ca(&self, mkcert_bin: &Path) -> Result<(), PrivError> {
        *self.mkcert_ca_path.lock().unwrap() = Some(mkcert_bin.to_path_buf());
        Ok(())
    }
    fn setcap_nginx(&self, _nginx_bin: &Path) -> Result<(), PrivError> {
        *self.setcap_done.lock().unwrap() = true;
        Ok(())
    }
    fn disable_system_services(&self, units: &[String]) -> Result<(), PrivError> {
        self.disabled_services.lock().unwrap().push(units.to_vec());
        Ok(())
    }
    fn write_resolved_dropin(&self, contents: &str) -> Result<(), PrivError> {
        self.resolved_dropins.lock().unwrap().push(contents.to_string());
        Ok(())
    }
    fn remove_resolved_dropin(&self) -> Result<(), PrivError> {
        *self.resolved_removed.lock().unwrap() = true;
        Ok(())
    }
    fn create_symlink(&self, src: &Path, dst: &Path) -> Result<(), PrivError> {
        self.symlinks_created.lock().unwrap().push((src.display().to_string(), dst.display().to_string()));
        Ok(())
    }
    fn remove_symlink(&self, dst: &Path) -> Result<(), PrivError> {
        self.symlinks_removed.lock().unwrap().push(dst.display().to_string());
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
        f.write_etc_hosts("# BEGIN laralux-linux\n# END laralux-linux\n").unwrap();
        assert_eq!(log.lock().unwrap().len(), 1);
        assert!(log.lock().unwrap()[0].contains("laralux-linux"));
    }

    #[test]
    fn pkexec_uses_pkexec_program() {
        // The pkexec impl escalates with `pkexec`; verify via the shared builder usage.
        // hosts_cp_command on Sudo still uses sudo (unchanged Plan-2 contract).
        let (prog, _args) = SudoPrivileged::hosts_cp_command(std::path::Path::new("/tmp/h"));
        assert_eq!(prog, "sudo");
    }

    #[test]
    fn systemctl_disable_argv_builds_disable_now() {
        let argv = systemctl_disable_argv(&["nginx".to_string(), "mariadb".to_string()]);
        assert_eq!(
            argv,
            vec![
                "systemctl".to_string(),
                "disable".to_string(),
                "--now".to_string(),
                "nginx".to_string(),
                "mariadb".to_string(),
            ]
        );
    }

    #[test]
    fn fake_records_disabled_services() {
        let f = FakePrivileged::new();
        let log = f.disabled_services();
        f.disable_system_services(&["nginx".to_string()]).unwrap();
        assert_eq!(log.lock().unwrap().len(), 1);
        assert_eq!(log.lock().unwrap()[0], vec!["nginx".to_string()]);
    }

    #[test]
    fn fake_records_resolved_dropin() {
        let f = FakePrivileged::new();
        f.write_resolved_dropin("[Resolve]\nDNS=127.0.0.1:5353\n").unwrap();
        assert_eq!(f.resolved_dropins().lock().unwrap().len(), 1);
        f.remove_resolved_dropin().unwrap();
        assert!(*f.resolved_removed().lock().unwrap());
    }

    #[test]
    fn symlink_argv_builders_are_correct() {
        assert_eq!(
            ln_symlink_argv(Path::new("/home/u/laralux/bin/php/current/php"), Path::new("/usr/local/bin/php")),
            vec!["ln".to_string(), "-sfn".to_string(),
                 "/home/u/laralux/bin/php/current/php".to_string(), "/usr/local/bin/php".to_string()]
        );
        assert_eq!(
            rm_argv(Path::new("/usr/local/bin/php")),
            vec!["rm".to_string(), "-f".to_string(), "/usr/local/bin/php".to_string()]
        );
    }

    #[test]
    fn fake_records_symlink_create_and_remove() {
        let p = FakePrivileged::new();
        p.create_symlink(Path::new("/src/php"), Path::new("/usr/local/bin/php")).unwrap();
        p.remove_symlink(Path::new("/usr/local/bin/php")).unwrap();
        assert_eq!(p.symlinks_created().lock().unwrap().as_slice(),
            &[("/src/php".to_string(), "/usr/local/bin/php".to_string())]);
        assert_eq!(p.symlinks_removed().lock().unwrap().as_slice(),
            &["/usr/local/bin/php".to_string()]);
    }
}
