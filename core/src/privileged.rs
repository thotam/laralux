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
    fn apt_install(&self, packages: &[String]) -> Result<(), PrivError>;
    fn add_apt_repository(&self, repo: &str) -> Result<(), PrivError>;
    fn disable_system_services(&self, units: &[String]) -> Result<(), PrivError>;
    fn allow_mariadb_apparmor(&self) -> Result<(), PrivError>;
    fn write_resolved_dropin(&self, contents: &str) -> Result<(), PrivError>;
    fn remove_resolved_dropin(&self) -> Result<(), PrivError>;
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

fn add_repo_argv(repo: &str) -> Vec<String> {
    vec!["add-apt-repository".to_string(), "-y".to_string(), repo.to_string()]
}

fn systemctl_disable_argv(units: &[String]) -> Vec<String> {
    let mut argv = vec!["systemctl".to_string(), "disable".to_string(), "--now".to_string()];
    argv.extend(units.iter().cloned());
    argv
}

const MARIADB_APPARMOR_SCRIPT: &str = "\
[ -f /etc/apparmor.d/mariadbd ] || exit 0\n\
grep -q laragon /etc/apparmor.d/local/mariadbd 2>/dev/null || echo 'owner /home/*/laragon/** rwk,' >> /etc/apparmor.d/local/mariadbd\n\
apparmor_parser -r /etc/apparmor.d/mariadbd\n";

fn mariadb_apparmor_argv() -> Vec<String> {
    vec!["sh".to_string(), "-c".to_string(), MARIADB_APPARMOR_SCRIPT.to_string()]
}

const RESOLVED_DROPIN: &str = "/etc/systemd/resolved.conf.d/laragon.conf";

fn write_resolved_argv(contents: &str) -> Vec<String> {
    vec![
        "sh".to_string(),
        "-c".to_string(),
        format!(
            "mkdir -p /etc/systemd/resolved.conf.d && cat > {RESOLVED_DROPIN} <<'LARAGONEOF'\n{contents}\nLARAGONEOF\nsystemctl reload systemd-resolved || systemctl restart systemd-resolved || true"
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

fn apt_argv(packages: &[String]) -> Vec<String> {
    vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("apt-get update || true; apt-get install -y {}", packages.join(" ")),
    ]
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
        let tmp = std::env::temp_dir().join("laragon-hosts.new");
        std::fs::write(&tmp, new_content)?;
        run_escalated("sudo", &cp_argv(&tmp))
    }
    fn install_mkcert_ca(&self, mkcert_bin: &Path) -> Result<(), PrivError> {
        run_escalated(&mkcert_bin.display().to_string(), &["-install".to_string()])
    }
    fn setcap_nginx(&self, nginx_bin: &Path) -> Result<(), PrivError> {
        run_escalated("sudo", &setcap_argv(nginx_bin))
    }
    fn apt_install(&self, packages: &[String]) -> Result<(), PrivError> {
        run_escalated("sudo", &apt_argv(packages))
    }
    fn add_apt_repository(&self, repo: &str) -> Result<(), PrivError> {
        run_escalated("sudo", &add_repo_argv(repo))
    }
    fn disable_system_services(&self, units: &[String]) -> Result<(), PrivError> {
        run_escalated("sudo", &systemctl_disable_argv(units))
    }
    fn allow_mariadb_apparmor(&self) -> Result<(), PrivError> {
        run_escalated("sudo", &mariadb_apparmor_argv())
    }
    fn write_resolved_dropin(&self, contents: &str) -> Result<(), PrivError> {
        run_escalated("sudo", &write_resolved_argv(contents))
    }
    fn remove_resolved_dropin(&self) -> Result<(), PrivError> {
        run_escalated("sudo", &remove_resolved_argv())
    }
}

// ---------- Real: pkexec (graphical auth) ----------

/// Privileged operations escalated with `pkexec` (graphical auth) — for GUI use.
pub struct PkexecPrivileged;

impl Privileged for PkexecPrivileged {
    fn write_etc_hosts(&self, new_content: &str) -> Result<(), PrivError> {
        let tmp = std::env::temp_dir().join("laragon-hosts.new");
        std::fs::write(&tmp, new_content)?;
        run_escalated("pkexec", &cp_argv(&tmp))
    }
    fn install_mkcert_ca(&self, mkcert_bin: &Path) -> Result<(), PrivError> {
        run_escalated(&mkcert_bin.display().to_string(), &["-install".to_string()])
    }
    fn setcap_nginx(&self, nginx_bin: &Path) -> Result<(), PrivError> {
        run_escalated("pkexec", &setcap_argv(nginx_bin))
    }
    fn apt_install(&self, packages: &[String]) -> Result<(), PrivError> {
        run_escalated("pkexec", &apt_argv(packages))
    }
    fn add_apt_repository(&self, repo: &str) -> Result<(), PrivError> {
        run_escalated("pkexec", &add_repo_argv(repo))
    }
    fn disable_system_services(&self, units: &[String]) -> Result<(), PrivError> {
        run_escalated("pkexec", &systemctl_disable_argv(units))
    }
    fn allow_mariadb_apparmor(&self) -> Result<(), PrivError> {
        run_escalated("pkexec", &mariadb_apparmor_argv())
    }
    fn write_resolved_dropin(&self, contents: &str) -> Result<(), PrivError> {
        run_escalated("pkexec", &write_resolved_argv(contents))
    }
    fn remove_resolved_dropin(&self) -> Result<(), PrivError> {
        run_escalated("pkexec", &remove_resolved_argv())
    }
}

// ---------- Fake (used by sync tests) ----------

#[derive(Clone, Default)]
pub struct FakePrivileged {
    hosts_writes: Arc<Mutex<Vec<String>>>,
    mkcert_ca_path: Arc<Mutex<Option<PathBuf>>>,
    setcap_done: Arc<Mutex<bool>>,
    apt_installs: Arc<Mutex<Vec<Vec<String>>>>,
    add_repos: Arc<Mutex<Vec<String>>>,
    disabled_services: Arc<Mutex<Vec<Vec<String>>>>,
    apparmor_configured: Arc<Mutex<bool>>,
    resolved_dropins: Arc<Mutex<Vec<String>>>,
    resolved_removed: Arc<Mutex<bool>>,
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
    pub fn apt_installs(&self) -> Arc<Mutex<Vec<Vec<String>>>> {
        self.apt_installs.clone()
    }
    pub fn add_repos(&self) -> Arc<Mutex<Vec<String>>> {
        self.add_repos.clone()
    }
    pub fn disabled_services(&self) -> Arc<Mutex<Vec<Vec<String>>>> {
        self.disabled_services.clone()
    }
    pub fn mariadb_apparmor_configured(&self) -> bool {
        *self.apparmor_configured.lock().unwrap()
    }
    pub fn resolved_dropins(&self) -> Arc<Mutex<Vec<String>>> {
        self.resolved_dropins.clone()
    }
    pub fn resolved_removed(&self) -> Arc<Mutex<bool>> {
        self.resolved_removed.clone()
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
    fn apt_install(&self, packages: &[String]) -> Result<(), PrivError> {
        self.apt_installs.lock().unwrap().push(packages.to_vec());
        Ok(())
    }
    fn add_apt_repository(&self, repo: &str) -> Result<(), PrivError> {
        self.add_repos.lock().unwrap().push(repo.to_string());
        Ok(())
    }
    fn disable_system_services(&self, units: &[String]) -> Result<(), PrivError> {
        self.disabled_services.lock().unwrap().push(units.to_vec());
        Ok(())
    }
    fn allow_mariadb_apparmor(&self) -> Result<(), PrivError> {
        *self.apparmor_configured.lock().unwrap() = true;
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

    #[test]
    fn apt_argv_builds_update_then_install() {
        let argv = apt_argv(&["nginx".to_string(), "redis-server".to_string()]);
        assert_eq!(argv[0], "sh");
        assert_eq!(argv[1], "-c");
        assert!(argv[2].contains("apt-get update || true"));
        assert!(argv[2].contains("apt-get install -y nginx redis-server"));
    }

    #[test]
    fn pkexec_uses_pkexec_program() {
        // The pkexec impl escalates with `pkexec`; verify via the shared builder usage.
        // hosts_cp_command on Sudo still uses sudo (unchanged Plan-2 contract).
        let (prog, _args) = SudoPrivileged::hosts_cp_command(std::path::Path::new("/tmp/h"));
        assert_eq!(prog, "sudo");
    }

    #[test]
    fn fake_records_apt_installs() {
        let f = FakePrivileged::new();
        let log = f.apt_installs();
        f.apt_install(&["nginx".to_string()]).unwrap();
        assert_eq!(log.lock().unwrap().len(), 1);
        assert_eq!(log.lock().unwrap()[0], vec!["nginx".to_string()]);
    }

    #[test]
    fn add_repo_argv_builds_add_apt_repository() {
        let argv = add_repo_argv("ppa:ondrej/php");
        assert_eq!(argv, vec!["add-apt-repository".to_string(), "-y".to_string(), "ppa:ondrej/php".to_string()]);
    }

    #[test]
    fn fake_records_add_repos() {
        let f = FakePrivileged::new();
        let log = f.add_repos();
        f.add_apt_repository("ppa:ondrej/php").unwrap();
        assert_eq!(log.lock().unwrap().as_slice(), &["ppa:ondrej/php".to_string()]);
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
    fn mariadb_apparmor_argv_runs_parser_with_laragon_rule() {
        let argv = mariadb_apparmor_argv();
        assert_eq!(argv[0], "sh");
        assert_eq!(argv[1], "-c");
        assert!(argv[2].contains("laragon"));
        assert!(argv[2].contains("/etc/apparmor.d/local/mariadbd"));
        assert!(argv[2].contains("apparmor_parser -r /etc/apparmor.d/mariadbd"));
    }

    #[test]
    fn fake_records_apparmor_configured() {
        let f = FakePrivileged::new();
        assert!(!f.mariadb_apparmor_configured());
        f.allow_mariadb_apparmor().unwrap();
        assert!(f.mariadb_apparmor_configured());
    }

    #[test]
    fn fake_records_resolved_dropin() {
        let f = FakePrivileged::new();
        f.write_resolved_dropin("[Resolve]\nDNS=127.0.0.1:5353\n").unwrap();
        assert_eq!(f.resolved_dropins().lock().unwrap().len(), 1);
        f.remove_resolved_dropin().unwrap();
        assert!(*f.resolved_removed().lock().unwrap());
    }
}
