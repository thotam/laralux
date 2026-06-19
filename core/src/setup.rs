use crate::bin::resolve_bin;
use crate::paths::LaragonPaths;
use serde::Serialize;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
pub enum Component {
    Nginx,
    Php,
    Mariadb,
    Redis,
    Mkcert,
    Mailpit,
}

impl Component {
    pub const ALL: [Component; 6] = [
        Component::Nginx,
        Component::Php,
        Component::Mariadb,
        Component::Redis,
        Component::Mkcert,
        Component::Mailpit,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Component::Nginx => "nginx",
            Component::Php => "php-fpm",
            Component::Mariadb => "mariadb",
            Component::Redis => "redis",
            Component::Mkcert => "mkcert",
            Component::Mailpit => "mailpit",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ComponentStatus {
    pub component: Component,
    pub present: bool,
}

/// The binary that, if resolvable, means the component is installed.
fn detect_binary(component: Component) -> String {
    match component {
        Component::Nginx => "nginx".to_string(),
        Component::Php => "php-fpm".to_string(), // unused for detection (handled in detect)
        Component::Mariadb => "mariadbd".to_string(),
        Component::Redis => "redis-server".to_string(),
        Component::Mkcert => "mkcert".to_string(),
        Component::Mailpit => "mailpit".to_string(),
    }
}

/// Detect presence of every component. Mailpit also searches `~/laragon/bin`.
pub fn detect(paths: &LaragonPaths) -> Vec<ComponentStatus> {
    Component::ALL
        .iter()
        .map(|&component| {
            let present = match component {
                Component::Php => crate::bin::detect_php_fpm_version(&[paths.bin()]).is_some(),
                other => {
                    let name = detect_binary(other);
                    resolve_bin(&name, &[paths.bin()]).is_some()
                }
            };
            ComponentStatus { component, present }
        })
        .collect()
}

/// The apt packages that install a component (empty for mailpit, which is downloaded).
pub fn apt_packages_for(component: Component) -> Vec<String> {
    match component {
        Component::Nginx => vec!["nginx".to_string()],
        Component::Php => vec![
            "php-fpm".to_string(),
            "php-cli".to_string(),
            "php-mysql".to_string(),
            "php-curl".to_string(),
            "php-mbstring".to_string(),
            "php-xml".to_string(),
        ],
        Component::Mariadb => vec!["mariadb-server".to_string()],
        Component::Redis => vec!["redis-server".to_string()],
        Component::Mkcert => vec!["mkcert".to_string(), "libnss3-tools".to_string()],
        Component::Mailpit => Vec::new(),
    }
}

use crate::privileged::Privileged;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub const MAILPIT_URL: &str =
    "https://github.com/axllent/mailpit/releases/latest/download/mailpit-linux-amd64.tar.gz";

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("setup io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("download error: {0}")]
    Download(String),
}

/// Fetches a URL to a destination file.
pub trait Downloader: Send + Sync {
    fn fetch(&self, url: &str, dest: &Path) -> Result<(), SetupError>;
}

pub struct CurlDownloader;

impl Downloader for CurlDownloader {
    fn fetch(&self, url: &str, dest: &Path) -> Result<(), SetupError> {
        let status = std::process::Command::new("curl")
            .arg("-fL")
            .arg(url)
            .arg("-o")
            .arg(dest)
            .status()
            .map_err(|e| SetupError::Download(format!("spawn curl: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(SetupError::Download(format!("curl failed for {url}")))
        }
    }
}

#[derive(Clone, Default)]
pub struct FakeDownloader {
    requested: Arc<Mutex<Vec<String>>>,
}

impl FakeDownloader {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn requested(&self) -> Arc<Mutex<Vec<String>>> {
        self.requested.clone()
    }
}

impl Downloader for FakeDownloader {
    fn fetch(&self, url: &str, dest: &Path) -> Result<(), SetupError> {
        self.requested.lock().unwrap().push(url.to_string());
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(dest, b"fake")?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SetupReport {
    pub apt_packages: Vec<String>,
    pub mailpit_fetched: bool,
    pub mkcert_ca: bool,
    pub nginx_setcap: bool,
    pub php_version: Option<String>,
    pub errors: Vec<String>,
}

/// Install missing components, fetch mailpit, install the mkcert CA, and setcap nginx.
/// Non-fatal: each failure is collected into `report.errors`.
pub fn run_setup(
    paths: &LaragonPaths,
    privileged: &dyn Privileged,
    downloader: &dyn Downloader,
) -> SetupReport {
    let mut report = SetupReport {
        apt_packages: Vec::new(),
        mailpit_fetched: false,
        mkcert_ca: false,
        nginx_setcap: false,
        php_version: None,
        errors: Vec::new(),
    };
    let _ = paths.ensure_dirs();
    let statuses = detect(paths);
    let missing: Vec<Component> =
        statuses.iter().filter(|s| !s.present).map(|s| s.component).collect();

    // 1. Install missing apt components: core stack in one call, PHP (unversioned
    //    distro meta) in a separate call so one failing package can't block the rest.
    let other_packages: Vec<String> = missing
        .iter()
        .filter(|c| !matches!(c, Component::Php))
        .flat_map(|&c| apt_packages_for(c))
        .collect();
    let php_packages: Vec<String> = missing
        .iter()
        .filter(|c| matches!(c, Component::Php))
        .flat_map(|&c| apt_packages_for(c))
        .collect();
    report.apt_packages = other_packages.iter().chain(php_packages.iter()).cloned().collect();

    if !other_packages.is_empty() {
        if let Err(e) = privileged.apt_install(&other_packages) {
            report.errors.push(format!("apt_install (core): {e}"));
        }
    }
    if !php_packages.is_empty() {
        if let Err(e) = privileged.apt_install(&php_packages) {
            report.errors.push(format!("apt_install (php): {e}"));
        }
        // Detect the version that actually got installed and persist it.
        match crate::bin::detect_php_fpm_version(&[paths.bin()]) {
            Some(ver) => {
                report.php_version = Some(ver.clone());
                let mut cfg = crate::config::Config::load(&paths.config_file()).unwrap_or_default();
                cfg.php_version = ver;
                if let Err(e) = cfg.save(&paths.config_file()) {
                    report.errors.push(format!("persist php version: {e}"));
                }
            }
            None => {
                report.errors.push(
                    "php-fpm binary not found after install; Config php_version not updated".to_string(),
                );
            }
        }
    }

    // 2. Fetch + extract mailpit into ~/laragon/bin when missing.
    if missing.contains(&Component::Mailpit) {
        let tarball = paths.tmp().join("mailpit.tar.gz");
        match downloader.fetch(MAILPIT_URL, &tarball) {
            Ok(()) => {
                report.mailpit_fetched = true;
                let output = std::process::Command::new("tar")
                    .arg("-xzf")
                    .arg(&tarball)
                    .arg("-C")
                    .arg(paths.bin())
                    .arg("mailpit")
                    .output();
                match output {
                    Ok(o) if o.status.success() => {}
                    Ok(o) => report.errors.push(format!(
                        "tar extract mailpit failed: {}",
                        String::from_utf8_lossy(&o.stderr).trim()
                    )),
                    Err(e) => report.errors.push(format!("tar spawn: {e}")),
                }
            }
            Err(e) => report.errors.push(format!("mailpit download: {e}")),
        }
    }

    // 3. Install the mkcert local CA (idempotent).
    match privileged.install_mkcert_ca() {
        Ok(()) => report.mkcert_ca = true,
        Err(e) => report.errors.push(format!("mkcert -install: {e}")),
    }

    // 4. setcap the resolved nginx binary (same path the orchestrator spawns).
    if let Some(nginx) = resolve_bin("nginx", &[paths.bin()]) {
        match privileged.setcap_nginx(&nginx) {
            Ok(()) => report.nginx_setcap = true,
            Err(e) => report.errors.push(format!("setcap nginx: {e}")),
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaragonPaths;
    use crate::privileged::FakePrivileged;

    #[test]
    fn php_packages_are_unversioned_meta() {
        let pkgs = apt_packages_for(Component::Php);
        assert!(pkgs.contains(&"php-fpm".to_string()));
        assert!(pkgs.contains(&"php-mysql".to_string()));
        assert!(!pkgs.iter().any(|p| p.contains('8'))); // no hardcoded version
    }

    #[test]
    fn mailpit_has_no_apt_packages() {
        assert!(apt_packages_for(Component::Mailpit).is_empty());
    }

    #[test]
    fn mkcert_includes_nss_tools() {
        let pkgs = apt_packages_for(Component::Mkcert);
        assert!(pkgs.contains(&"mkcert".to_string()));
        assert!(pkgs.contains(&"libnss3-tools".to_string()));
    }

    #[test]
    fn detect_reports_all_components() {
        let paths = LaragonPaths::new(std::env::temp_dir().join(format!("lara-detect-{}", std::process::id())));
        let statuses = detect(&paths);
        assert_eq!(statuses.len(), 6);
        assert!(!statuses.iter().find(|s| s.component == Component::Mailpit).unwrap().present);
    }

    #[test]
    fn run_setup_installs_core_and_php_without_ppa() {
        let root = std::env::temp_dir().join(format!("lara-runsetup-{}", std::process::id()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let paths = LaragonPaths::new(root.clone());

        let priv_ = FakePrivileged::new();
        let apt_log = priv_.apt_installs();
        let add_repos = priv_.add_repos();
        let dl = FakeDownloader::new();
        let urls = dl.requested();

        let report = run_setup(&paths, &priv_, &dl);

        // No PPA is added anymore.
        assert!(add_repos.lock().unwrap().is_empty());
        // Two apt installs: core (has nginx, no php) + php (unversioned meta).
        let calls = apt_log.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let core = calls.iter().find(|c| c.iter().any(|p| p == "nginx")).unwrap();
        assert!(core.iter().any(|p| p == "mariadb-server"));
        assert!(!core.iter().any(|p| p.starts_with("php")));
        let php = calls.iter().find(|c| c.iter().all(|p| p.starts_with("php"))).unwrap();
        assert!(php.iter().any(|p| p == "php-fpm"));
        // mailpit fetched, mkcert CA attempted.
        assert!(urls.lock().unwrap().iter().any(|u| u.contains("mailpit")));
        assert!(report.mkcert_ca);
        std::fs::remove_dir_all(&root).ok();
    }
}
