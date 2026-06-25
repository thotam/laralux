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
    Composer,
}

impl Component {
    pub const ALL: [Component; 7] = [
        Component::Nginx,
        Component::Php,
        Component::Mariadb,
        Component::Redis,
        Component::Mkcert,
        Component::Mailpit,
        Component::Composer,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Component::Nginx => "nginx",
            Component::Php => "php-fpm",
            Component::Mariadb => "mariadb",
            Component::Redis => "redis",
            Component::Mkcert => "mkcert",
            Component::Mailpit => "mailpit",
            Component::Composer => "composer",
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
        Component::Composer => "composer".to_string(),
    }
}

/// Detect presence of every component. Mailpit also searches `~/laragon/bin`.
pub fn detect(paths: &LaragonPaths) -> Vec<ComponentStatus> {
    Component::ALL
        .iter()
        .map(|&component| {
            let present = match component {
                Component::Php => crate::bin::resolve_bin("php-fpm", &crate::layout::managed_bin_dirs(paths)).is_some(),
                Component::Composer => crate::bin::resolve_bin("composer", &crate::layout::managed_bin_dirs(paths)).is_some(),
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
        Component::Php => Vec::new(),
        Component::Mariadb => vec!["mariadb-server".to_string()],
        Component::Redis => vec!["redis-server".to_string()],
        Component::Mkcert => vec!["mkcert".to_string(), "libnss3-tools".to_string()],
        Component::Mailpit => Vec::new(),
        Component::Composer => Vec::new(),
    }
}

use crate::privileged::Privileged;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub const MAILPIT_URL: &str =
    "https://github.com/axllent/mailpit/releases/latest/download/mailpit-linux-amd64.tar.gz";

pub const MAILPIT_FALLBACK_VERSION: &str = "1.20.0";

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
    pub composer_fetched: bool,
    pub mkcert_ca: bool,
    pub nginx_setcap: bool,
    pub php_version: Option<String>,
    pub errors: Vec<String>,
}

/// Distro systemd stack units to disable: nginx, mariadb, redis-server.
fn stack_units_to_disable() -> Vec<String> {
    vec!["nginx".to_string(), "mariadb".to_string(), "redis-server".to_string()]
}

/// Install missing components, fetch mailpit, install the mkcert CA, and setcap nginx.
/// Non-fatal: each failure is collected into `report.errors`.
pub fn run_setup(
    paths: &LaragonPaths,
    privileged: &dyn Privileged,
    downloader: &dyn Downloader,
    runner: &dyn crate::scaffold::CommandRunner,
) -> SetupReport {
    let mut report = SetupReport {
        apt_packages: Vec::new(),
        mailpit_fetched: false,
        composer_fetched: false,
        mkcert_ca: false,
        nginx_setcap: false,
        php_version: None,
        errors: Vec::new(),
    };
    let _ = paths.ensure_dirs();
    let statuses = detect(paths);
    let missing: Vec<Component> =
        statuses.iter().filter(|s| !s.present).map(|s| s.component).collect();

    // 1. Install missing apt components (core stack only; PHP is static, below).
    let apt_packages: Vec<String> =
        missing.iter().flat_map(|&c| apt_packages_for(c)).collect();
    report.apt_packages = apt_packages.clone();
    if !apt_packages.is_empty() {
        if let Err(e) = privileged.apt_install(&apt_packages) {
            report.errors.push(format!("apt_install: {e}"));
        }
    }

    // 1b. Install PHP from a static build (no apt/distro PHP) when missing.
    if missing.contains(&Component::Php) {
        match crate::php_static::install_php_static(paths, crate::php_versions::DEFAULT_PHP_VERSION, downloader, runner) {
            Ok(full) => {
                report.php_version = Some(full.clone());
                let mut cfg = crate::config::Config::load(&paths.config_file()).unwrap_or_default();
                cfg.versions.insert("php".to_string(), full.clone());
                cfg.php_version = full.clone();
                if let Err(e) = cfg.save(&paths.config_file()) {
                    report.errors.push(format!("persist php version: {e}"));
                }
            }
            Err(e) => report.errors.push(format!("install php (static): {e}")),
        }
    }

    // 1c. Install composer (downloaded, not apt) when missing. Must run after PHP so
    // the PHP binary is available for probing the composer version.
    if missing.contains(&Component::Composer) {
        if let Err(e) = crate::php_cli::install_composer(paths, downloader) {
            report.errors.push(format!("install composer: {e}"));
        } else {
            report.composer_fetched = true;
        }
    }

    // apt auto-starts + enables the distro nginx/mariadb/redis systemd units, which
    // hold ports 80/3306/6379. Disable them so the app-managed processes can bind.
    let stack_units = stack_units_to_disable();
    if let Err(e) = privileged.disable_system_services(&stack_units) {
        report.errors.push(format!("disable system services: {e}"));
    }

    // 2. Fetch + extract mailpit into ~/laragon/bin/<version>/ when missing.
    if missing.contains(&Component::Mailpit) {
        let tarball = paths.tmp().join("mailpit.tar.gz");
        match downloader.fetch(MAILPIT_URL, &tarball) {
            Ok(()) => {
                report.mailpit_fetched = true;
                let extract_dir = paths.tmp().join("mailpit-extract");
                let _ = std::fs::create_dir_all(&extract_dir);
                let out = std::process::Command::new("tar")
                    .arg("-xzf").arg(&tarball).arg("-C").arg(&extract_dir).arg("mailpit").output();
                match out {
                    Ok(o) if o.status.success() => {
                        let extracted = extract_dir.join("mailpit");
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = std::fs::set_permissions(&extracted, std::fs::Permissions::from_mode(0o755));
                        }
                        let ver = crate::layout::probe_version(&extracted, &["version"])
                            .unwrap_or_else(|| MAILPIT_FALLBACK_VERSION.to_string());
                        let dir = paths.version_dir("mailpit", &ver);
                        let _ = std::fs::create_dir_all(&dir);
                        let dest = dir.join("mailpit");
                        let moved = std::fs::rename(&extracted, &dest).or_else(|_| {
                            std::fs::copy(&extracted, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(&extracted))
                        });
                        match moved {
                            Ok(()) => {
                                let _ = crate::layout::set_current(paths, "mailpit", &ver);
                                let mut cfg = crate::config::Config::load(&paths.config_file()).unwrap_or_default();
                                cfg.versions.insert("mailpit".to_string(), ver);
                                let _ = cfg.save(&paths.config_file());
                            }
                            Err(e) => report.errors.push(format!("install mailpit: {e}")),
                        }
                    }
                    Ok(o) => report.errors.push(format!("tar extract mailpit failed: {}", String::from_utf8_lossy(&o.stderr).trim())),
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
    if let Some(nginx) = resolve_bin("nginx", &crate::layout::managed_bin_dirs(paths)) {
        match privileged.setcap_nginx(&nginx) {
            Ok(()) => report.nginx_setcap = true,
            Err(e) => report.errors.push(format!("setcap nginx: {e}")),
        }
    }

    // Ubuntu's mariadb AppArmor profile confines mariadbd to standard paths;
    // allow it into ~/laragon so the app-managed datadir works.
    if let Err(e) = privileged.allow_mariadb_apparmor() {
        report.errors.push(format!("mariadb apparmor: {e}"));
    }

    // Reconcile all `current` symlinks from the freshly-written config.
    let cfg = crate::config::Config::load(&paths.config_file()).unwrap_or_default();
    for w in crate::layout::apply_versions(paths, &cfg) {
        report.errors.push(format!("apply versions: {w}"));
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaragonPaths;
    use crate::privileged::FakePrivileged;

    #[test]
    fn apt_packages_for_php_is_empty() {
        assert!(apt_packages_for(Component::Php).is_empty());
    }

    #[test]
    fn stack_units_exclude_php() {
        assert_eq!(
            stack_units_to_disable(),
            vec!["nginx".to_string(), "mariadb".to_string(), "redis-server".to_string()]
        );
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
        assert_eq!(statuses.len(), 7);
        assert!(!statuses.iter().find(|s| s.component == Component::Mailpit).unwrap().present);
    }

    #[test]
    fn run_setup_disables_distro_stack_services() {
        let root = std::env::temp_dir().join(format!("lara-disable-{}", std::process::id()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let paths = LaragonPaths::new(root.clone());
        let priv_ = FakePrivileged::new();
        let disabled = priv_.disabled_services();
        let dl = FakeDownloader::new();
        let runner = crate::scaffold::FakeCommandRunner::new();

        let _ = run_setup(&paths, &priv_, &dl, &runner);

        let calls = disabled.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let units = &calls[0];
        assert!(units.contains(&"nginx".to_string()));
        assert!(units.contains(&"mariadb".to_string()));
        assert!(units.contains(&"redis-server".to_string()));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn run_setup_adds_no_ppa() {
        let root = std::env::temp_dir().join(format!("lara-runsetup-{}", std::process::id()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let paths = LaragonPaths::new(root.clone());
        let priv_ = FakePrivileged::new();
        let add_repos = priv_.add_repos();
        let dl = FakeDownloader::new();
        let runner = crate::scaffold::FakeCommandRunner::new();
        let _ = run_setup(&paths, &priv_, &dl, &runner);
        // Hermetic: regardless of what's installed, we never add a PPA.
        assert!(add_repos.lock().unwrap().is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn run_setup_configures_mariadb_apparmor() {
        let root = std::env::temp_dir().join(format!("lara-aa-{}", std::process::id()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let paths = LaragonPaths::new(root.clone());
        let priv_ = FakePrivileged::new();
        let dl = FakeDownloader::new();
        let runner = crate::scaffold::FakeCommandRunner::new();
        let _ = run_setup(&paths, &priv_, &dl, &runner);
        assert!(priv_.mariadb_apparmor_configured());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn composer_is_a_component_with_no_apt_package() {
        assert!(Component::ALL.contains(&Component::Composer));
        assert!(apt_packages_for(Component::Composer).is_empty());
        assert_eq!(Component::Composer.label(), "composer");
    }
}
