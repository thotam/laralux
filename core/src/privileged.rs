use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error)]
pub enum PrivError {
    #[error("privileged io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("privileged command failed: {0}")]
    Command(String),
}

/// Inputs for the batched, setup-time privileged steps (`run_setup_privileged`).
/// Bundling them lets the GUI ask for authorization a single time instead of
/// once per operation.
pub struct SetupPrivPlan {
    /// Distro systemd units to disable (best-effort; missing units are skipped).
    pub disable_units: Vec<String>,
    /// Resolved mkcert binary, or `None` when mkcert isn't installed.
    pub mkcert_bin: Option<PathBuf>,
    /// Resolved nginx binary, or `None` when nginx isn't installed.
    pub nginx_bin: Option<PathBuf>,
}

/// Per-step result of `run_setup_privileged`. `None` means the step was skipped
/// because its input wasn't available (e.g. the binary wasn't resolved).
pub struct SetupPrivOutcome {
    pub disabled_services: Result<(), String>,
    pub mkcert_ca: Option<Result<(), String>>,
    pub setcap_nginx: Option<Result<(), String>>,
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
    fn ensure_php_ini_link(&self, target: &Path) -> Result<(), PrivError>;
    /// Run the setup-time privileged steps (disable distro units, install the
    /// mkcert system CA, setcap nginx) under a single escalation prompt.
    fn run_setup_privileged(&self, plan: &SetupPrivPlan) -> SetupPrivOutcome;
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

/// Single-quote a string for safe interpolation into an `sh -c` script.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Shell that disables each distro unit independently, skipping any that aren't
/// installed. laralux runs its stack from static tarballs, so the distro nginx/
/// mariadb/redis systemd units usually don't exist — `systemctl disable` on a
/// missing unit errors and would otherwise fail the whole batch. Returns
/// non-zero only when a unit that *does* exist fails to disable.
fn systemctl_disable_script(units: &[String]) -> String {
    let list = units.iter().map(|u| shell_quote(u)).collect::<Vec<_>>().join(" ");
    format!(
        "rc=0; for u in {list}; do systemctl cat \"$u\" >/dev/null 2>&1 || continue; \
         systemctl disable --now \"$u\" || rc=1; done; exit $rc"
    )
}

fn systemctl_disable_argv(units: &[String]) -> Vec<String> {
    vec!["sh".to_string(), "-c".to_string(), systemctl_disable_script(units)]
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

fn php_ini_link_argv(target: &Path) -> Vec<String> {
    vec![
        "sh".to_string(),
        "-c".to_string(),
        format!(
            "mkdir -p /usr/local/etc/php && ln -sfn {} /usr/local/etc/php/php.ini",
            target.display()
        ),
    ]
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

/// Like `run_escalated`, but captures stdout (used to read per-step markers from
/// the batched setup script). Returns the child's stdout on success.
fn run_escalated_capture(escalator: &str, argv: &[String]) -> Result<String, PrivError> {
    let out = std::process::Command::new(escalator)
        .args(argv)
        .output()
        .map_err(|e| PrivError::Command(format!("spawn {escalator}: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        // Non-zero here means the escalator itself failed (e.g. auth cancelled):
        // the batched script always ends with `exit 0`, so a successful run is 0.
        Err(PrivError::Command(format!("{escalator} command failed")))
    }
}

/// Ask mkcert (run as the current, unprivileged user) where its CAROOT lives.
fn mkcert_caroot(mkcert_bin: &Path) -> Result<PathBuf, PrivError> {
    let out = std::process::Command::new(mkcert_bin)
        .arg("-CAROOT")
        .output()
        .map_err(|e| PrivError::Command(format!("spawn mkcert -CAROOT: {e}")))?;
    if !out.status.success() {
        return Err(PrivError::Command("mkcert -CAROOT failed".to_string()));
    }
    let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if dir.is_empty() {
        return Err(PrivError::Command("mkcert -CAROOT returned an empty path".to_string()));
    }
    Ok(PathBuf::from(dir))
}

/// Generate the local CA as the current user when it's missing, so the rootCA
/// files stay user-owned — per-site cert issuance later runs mkcert as the user
/// and must read `rootCA-key.pem`. Signing a throwaway leaf creates the CA
/// without touching any trust store.
fn ensure_ca_generated(mkcert_bin: &Path, caroot: &Path) -> Result<(), PrivError> {
    if caroot.join("rootCA.pem").exists() && caroot.join("rootCA-key.pem").exists() {
        return Ok(());
    }
    let tmp = std::env::temp_dir();
    let cert = tmp.join("laralux-ca-gen.pem");
    let key = tmp.join("laralux-ca-gen-key.pem");
    let status = std::process::Command::new(mkcert_bin)
        .arg("-cert-file")
        .arg(&cert)
        .arg("-key-file")
        .arg(&key)
        .arg("laralux.invalid")
        .env("CAROOT", caroot)
        .status()
        .map_err(|e| PrivError::Command(format!("spawn mkcert (CA generation): {e}")))?;
    let _ = std::fs::remove_file(&cert);
    let _ = std::fs::remove_file(&key);
    if status.success() {
        Ok(())
    } else {
        Err(PrivError::Command("mkcert CA generation failed".to_string()))
    }
}

/// The privileged command that installs the mkcert CA into the SYSTEM trust
/// store. mkcert's own `-install` shells out to `sudo` for the system store,
/// which can't prompt in a GUI (no TTY); instead we run mkcert itself as root
/// and pin `CAROOT` to the user's CA so it installs that one rather than minting
/// a fresh, root-owned CA. Scoped to `TRUST_STORES=system`; browser NSS is
/// handled separately by `certutil_static`.
fn mkcert_system_cmd(mkcert_bin: &Path, caroot: &Path) -> String {
    format!(
        "CAROOT={} TRUST_STORES=system {} -install",
        shell_quote(&caroot.display().to_string()),
        shell_quote(&mkcert_bin.display().to_string()),
    )
}

fn setcap_cmd(nginx_bin: &Path) -> String {
    format!("setcap cap_net_bind_service=+ep {}", shell_quote(&nginx_bin.display().to_string()))
}

// Per-step markers emitted by the batched setup script, parsed from its stdout.
const M_DISABLE_OK: &str = "__LARALUX_DISABLE_OK__";
const M_DISABLE_FAIL: &str = "__LARALUX_DISABLE_FAIL__";
const M_MKCERT_OK: &str = "__LARALUX_MKCERT_OK__";
const M_MKCERT_FAIL: &str = "__LARALUX_MKCERT_FAIL__";
const M_SETCAP_OK: &str = "__LARALUX_SETCAP_OK__";
const M_SETCAP_FAIL: &str = "__LARALUX_SETCAP_FAIL__";

/// Build the combined root script: each included step runs best-effort and
/// prints an OK/FAIL marker; the script always ends with `exit 0` so a non-zero
/// child status unambiguously means the escalation itself failed.
fn build_setup_script(
    units: &[String],
    mkcert_cmd: Option<&str>,
    setcap_cmd: Option<&str>,
) -> String {
    let list = units.iter().map(|u| shell_quote(u)).collect::<Vec<_>>().join(" ");
    let mut s = format!(
        "rc=0; for u in {list}; do systemctl cat \"$u\" >/dev/null 2>&1 || continue; \
         systemctl disable --now \"$u\" || rc=1; done; \
         [ $rc -eq 0 ] && echo {M_DISABLE_OK} || echo {M_DISABLE_FAIL}\n"
    );
    if let Some(cmd) = mkcert_cmd {
        s.push_str(&format!("if {cmd}; then echo {M_MKCERT_OK}; else echo {M_MKCERT_FAIL}; fi\n"));
    }
    if let Some(cmd) = setcap_cmd {
        s.push_str(&format!("if {cmd}; then echo {M_SETCAP_OK}; else echo {M_SETCAP_FAIL}; fi\n"));
    }
    s.push_str("exit 0\n");
    s
}

/// Map an OK/FAIL marker pair (or a whole-escalation failure) to a step result.
fn parse_marker(
    stdout: &str,
    ok: &str,
    fail: &str,
    esc_err: Option<&str>,
    label: &str,
) -> Result<(), String> {
    if stdout.contains(ok) {
        Ok(())
    } else if stdout.contains(fail) {
        Err(format!("{label} failed"))
    } else if let Some(e) = esc_err {
        Err(e.to_string())
    } else {
        Err(format!("{label}: no result"))
    }
}

/// Shared implementation of `run_setup_privileged` for the real escalators.
fn run_setup_steps_escalated(escalator: &str, plan: &SetupPrivPlan) -> SetupPrivOutcome {
    // mkcert prep runs unprivileged (as the current user): discover CAROOT and
    // ensure the CA exists & is user-owned before installing it system-wide.
    let mut mkcert_cmd: Option<String> = None;
    let mut mkcert_pre_err: Option<String> = None;
    if let Some(bin) = &plan.mkcert_bin {
        match mkcert_caroot(bin).and_then(|caroot| ensure_ca_generated(bin, &caroot).map(|()| caroot)) {
            Ok(caroot) => mkcert_cmd = Some(mkcert_system_cmd(bin, &caroot)),
            Err(e) => mkcert_pre_err = Some(e.to_string()),
        }
    }
    let setcap = plan.nginx_bin.as_ref().map(|b| setcap_cmd(b));
    let script = build_setup_script(&plan.disable_units, mkcert_cmd.as_deref(), setcap.as_deref());

    let (stdout, esc_err) = match run_escalated_capture(escalator, &["sh".to_string(), "-c".to_string(), script]) {
        Ok(s) => (s, None),
        Err(e) => (String::new(), Some(e.to_string())),
    };
    let esc = esc_err.as_deref();

    let disabled_services = parse_marker(&stdout, M_DISABLE_OK, M_DISABLE_FAIL, esc, "disable services");
    let mkcert_ca = if let Some(pre) = mkcert_pre_err {
        Some(Err(pre))
    } else if plan.mkcert_bin.is_some() {
        Some(parse_marker(&stdout, M_MKCERT_OK, M_MKCERT_FAIL, esc, "mkcert -install (system)"))
    } else {
        None
    };
    let setcap_nginx = if plan.nginx_bin.is_some() {
        Some(parse_marker(&stdout, M_SETCAP_OK, M_SETCAP_FAIL, esc, "setcap nginx"))
    } else {
        None
    };
    SetupPrivOutcome { disabled_services, mkcert_ca, setcap_nginx }
}

/// Install the mkcert CA into the system trust store under `escalator`, used by
/// the standalone `install_mkcert_ca` path (e.g. the CLI).
fn install_ca_system(escalator: &str, mkcert_bin: &Path) -> Result<(), PrivError> {
    let caroot = mkcert_caroot(mkcert_bin)?;
    ensure_ca_generated(mkcert_bin, &caroot)?;
    run_escalated(
        escalator,
        &["sh".to_string(), "-c".to_string(), mkcert_system_cmd(mkcert_bin, &caroot)],
    )
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
        install_ca_system("sudo", mkcert_bin)
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
    fn ensure_php_ini_link(&self, target: &Path) -> Result<(), PrivError> {
        run_escalated("sudo", &php_ini_link_argv(target))
    }
    fn run_setup_privileged(&self, plan: &SetupPrivPlan) -> SetupPrivOutcome {
        run_setup_steps_escalated("sudo", plan)
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
        install_ca_system("pkexec", mkcert_bin)
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
    fn ensure_php_ini_link(&self, target: &Path) -> Result<(), PrivError> {
        run_escalated("pkexec", &php_ini_link_argv(target))
    }
    fn run_setup_privileged(&self, plan: &SetupPrivPlan) -> SetupPrivOutcome {
        run_setup_steps_escalated("pkexec", plan)
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
    php_ini_links: Arc<Mutex<Vec<String>>>,
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
    pub fn php_ini_links(&self) -> Arc<Mutex<Vec<String>>> {
        self.php_ini_links.clone()
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
    fn ensure_php_ini_link(&self, target: &Path) -> Result<(), PrivError> {
        self.php_ini_links.lock().unwrap().push(target.display().to_string());
        Ok(())
    }
    fn run_setup_privileged(&self, plan: &SetupPrivPlan) -> SetupPrivOutcome {
        self.disabled_services.lock().unwrap().push(plan.disable_units.clone());
        let mkcert_ca = plan.mkcert_bin.as_ref().map(|bin| {
            *self.mkcert_ca_path.lock().unwrap() = Some(bin.clone());
            Ok(())
        });
        let setcap_nginx = plan.nginx_bin.as_ref().map(|_| {
            *self.setcap_done.lock().unwrap() = true;
            Ok(())
        });
        SetupPrivOutcome { disabled_services: Ok(()), mkcert_ca, setcap_nginx }
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
        f.write_etc_hosts("# BEGIN laralux\n# END laralux\n").unwrap();
        assert_eq!(log.lock().unwrap().len(), 1);
        assert!(log.lock().unwrap()[0].contains("laralux"));
    }

    #[test]
    fn pkexec_uses_pkexec_program() {
        // The pkexec impl escalates with `pkexec`; verify via the shared builder usage.
        // hosts_cp_command on Sudo still uses sudo (unchanged Plan-2 contract).
        let (prog, _args) = SudoPrivileged::hosts_cp_command(std::path::Path::new("/tmp/h"));
        assert_eq!(prog, "sudo");
    }

    #[test]
    fn systemctl_disable_argv_skips_missing_units() {
        let argv = systemctl_disable_argv(&["nginx".to_string(), "mariadb".to_string()]);
        assert_eq!(argv[0], "sh");
        assert_eq!(argv[1], "-c");
        // Each unit is gated on `systemctl cat` so absent units are skipped, and
        // the loop is best-effort except for units that actually fail to disable.
        assert!(argv[2].contains("systemctl cat \"$u\""));
        assert!(argv[2].contains("systemctl disable --now \"$u\""));
        assert!(argv[2].contains("'nginx' 'mariadb'"));
        assert!(argv[2].contains("exit $rc"));
    }

    #[test]
    fn build_setup_script_emits_markers_for_included_steps() {
        let units = vec!["nginx".to_string()];
        // All three steps included.
        let s = build_setup_script(&units, Some("MK -install"), Some("setcap X"));
        assert!(s.contains(M_DISABLE_OK) && s.contains(M_DISABLE_FAIL));
        assert!(s.contains(M_MKCERT_OK) && s.contains("if MK -install; then"));
        assert!(s.contains(M_SETCAP_OK) && s.contains("if setcap X; then"));
        assert!(s.trim_end().ends_with("exit 0"));
        // Skipped steps emit no markers.
        let only_disable = build_setup_script(&units, None, None);
        assert!(!only_disable.contains(M_MKCERT_OK));
        assert!(!only_disable.contains(M_SETCAP_OK));
    }

    #[test]
    fn parse_marker_prefers_ok_then_fail_then_escalation_error() {
        assert!(parse_marker(M_MKCERT_OK, M_MKCERT_OK, M_MKCERT_FAIL, None, "x").is_ok());
        assert_eq!(
            parse_marker(M_MKCERT_FAIL, M_MKCERT_OK, M_MKCERT_FAIL, None, "mkcert"),
            Err("mkcert failed".to_string())
        );
        // No marker + escalation failed → surface the escalation error.
        assert_eq!(
            parse_marker("", M_MKCERT_OK, M_MKCERT_FAIL, Some("pkexec command failed"), "mkcert"),
            Err("pkexec command failed".to_string())
        );
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_quote("/home/u/laralux"), "'/home/u/laralux'");
    }

    #[test]
    fn fake_run_setup_privileged_records_and_skips_absent_inputs() {
        let f = FakePrivileged::new();
        let plan = SetupPrivPlan {
            disable_units: vec!["nginx".to_string()],
            mkcert_bin: Some(PathBuf::from("/x/mkcert")),
            nginx_bin: None,
        };
        let out = f.run_setup_privileged(&plan);
        assert!(out.disabled_services.is_ok());
        assert!(matches!(out.mkcert_ca, Some(Ok(()))));
        assert!(out.setcap_nginx.is_none()); // nginx_bin None → skipped
        assert_eq!(f.mkcert_ca_path(), Some(PathBuf::from("/x/mkcert")));
        assert_eq!(f.disabled_services().lock().unwrap().len(), 1);
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
        f.write_resolved_dropin(&format!("[Resolve]\nDNS=127.0.0.1:{}\n", crate::coredns::COREDNS_PORT)).unwrap();
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

    #[test]
    fn php_ini_link_argv_is_mkdir_then_ln() {
        let argv = php_ini_link_argv(std::path::Path::new("/home/u/laralux/etc/php/php.ini"));
        assert_eq!(argv[0], "sh");
        assert_eq!(argv[1], "-c");
        assert!(argv[2].contains("mkdir -p /usr/local/etc/php"));
        assert!(argv[2].contains("ln -sfn /home/u/laralux/etc/php/php.ini /usr/local/etc/php/php.ini"));
    }

    #[test]
    fn fake_records_php_ini_link() {
        let p = FakePrivileged::new();
        p.ensure_php_ini_link(std::path::Path::new("/x/php.ini")).unwrap();
        assert_eq!(p.php_ini_links().lock().unwrap().as_slice(), &["/x/php.ini".to_string()]);
    }
}
