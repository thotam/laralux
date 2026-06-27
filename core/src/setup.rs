use crate::bin::resolve_bin;
use crate::paths::LaraluxPaths;
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
    Node,
}

impl Component {
    pub const ALL: [Component; 8] = [
        Component::Nginx,
        Component::Php,
        Component::Mariadb,
        Component::Redis,
        Component::Mkcert,
        Component::Mailpit,
        Component::Composer,
        Component::Node,
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
            Component::Node => "node",
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
        Component::Node => "node".to_string(),
    }
}

/// Detect presence of every component. Mailpit also searches `~/laralux/bin`.
pub fn detect(paths: &LaraluxPaths) -> Vec<ComponentStatus> {
    Component::ALL
        .iter()
        .map(|&component| {
            let present = match component {
                Component::Php => crate::bin::resolve_bin("php-fpm", &crate::layout::managed_bin_dirs(paths)).is_some(),
                Component::Composer => crate::bin::resolve_bin("composer", &crate::layout::managed_bin_dirs(paths)).is_some(),
                other => {
                    let name = detect_binary(other);
                    resolve_bin(&name, &crate::layout::managed_bin_dirs(paths)).is_some()
                }
            };
            ComponentStatus { component, present }
        })
        .collect()
}

/// The apt packages that install a component (empty for mailpit, which is downloaded).
pub fn apt_packages_for(component: Component) -> Vec<String> {
    match component {
        Component::Nginx => Vec::new(),
        Component::Php => Vec::new(),
        Component::Mariadb => Vec::new(),
        Component::Redis => Vec::new(),
        Component::Mkcert => Vec::new(),
        Component::Mailpit => Vec::new(),
        Component::Composer => Vec::new(),
        Component::Node => Vec::new(),
    }
}

use crate::privileged::Privileged;
use crate::progress::ProgressSink;
use std::path::Path;
use std::sync::{Arc, Mutex};


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
    /// Fetch while reporting byte progress to `sink`. Default: no byte progress.
    fn fetch_with_progress(&self, url: &str, dest: &Path, sink: &dyn ProgressSink) -> Result<(), SetupError> {
        let _ = sink;
        self.fetch(url, dest)
    }
}

/// Last `content-length` header value (case-insensitive) in a raw HTTP header
/// blob, or 0 if none/unparsable.
pub fn parse_content_length(headers: &str) -> u64 {
    let mut total = 0u64;
    for line in headers.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                if let Ok(n) = v.trim().parse::<u64>() {
                    total = n;
                }
            }
        }
    }
    total
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

    fn fetch_with_progress(&self, url: &str, dest: &Path, sink: &dyn ProgressSink) -> Result<(), SetupError> {
        use crate::progress::ProgressEvent;
        // Total size via a HEAD; 0 (unknown) is fine — the UI shows an indeterminate ring.
        let total = std::process::Command::new("curl")
            .args(["-sIL", url])
            .output()
            .ok()
            .map(|o| parse_content_length(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or(0);
        // Start the download in the background; poll the growing dest file for progress.
        let mut child = std::process::Command::new("curl")
            .arg("-fL").arg(url).arg("-o").arg(dest)
            .spawn()
            .map_err(|e| SetupError::Download(format!("spawn curl: {e}")))?;
        loop {
            let current = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
            sink.emit(ProgressEvent::Bytes { current, total });
            match child.try_wait() {
                Ok(Some(status)) => {
                    if status.success() {
                        let done = if total > 0 { total } else { current };
                        sink.emit(ProgressEvent::Bytes { current: done, total });
                        return Ok(());
                    }
                    return Err(SetupError::Download(format!("curl failed for {url}")));
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(150)),
                Err(e) => return Err(SetupError::Download(format!("curl wait: {e}"))),
            }
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
    pub nginx_fetched: bool,
    pub redis_fetched: bool,
    pub mkcert_fetched: bool,
    pub mkcert_ca: bool,
    pub certutil_fetched: bool,
    pub mkcert_nss: bool,
    pub nginx_setcap: bool,
    pub mariadb_fetched: bool,
    pub node_fetched: bool,
    pub php_version: Option<String>,
    pub errors: Vec<String>,
}

/// Distro systemd stack units to disable: nginx, mariadb, redis-server.
fn stack_units_to_disable() -> Vec<String> {
    vec!["nginx".to_string(), "mariadb".to_string(), "redis-server".to_string()]
}

/// Persist a tool version into config.versions (used by apply_versions for `current` symlinks).
fn record_version(paths: &LaraluxPaths, tool: &str, version: &str) {
    let mut cfg = crate::config::Config::load(&paths.config_file()).unwrap_or_default();
    cfg.versions.insert(tool.to_string(), version.to_string());
    let _ = cfg.save(&paths.config_file());
}

/// Install missing components, fetch mailpit, install the mkcert CA, and setcap nginx.
/// Non-fatal: each failure is collected into `report.errors`.
pub fn run_setup(
    paths: &LaraluxPaths,
    privileged: &dyn Privileged,
    downloader: &dyn Downloader,
    runner: &dyn crate::scaffold::CommandRunner,
    sink: &dyn crate::progress::ProgressSink,
) -> SetupReport {
    use crate::progress::ProgressEvent;
    let mut report = SetupReport {
        apt_packages: Vec::new(),
        mailpit_fetched: false,
        composer_fetched: false,
        nginx_fetched: false,
        redis_fetched: false,
        mkcert_fetched: false,
        mkcert_ca: false,
        certutil_fetched: false,
        mkcert_nss: false,
        nginx_setcap: false,
        mariadb_fetched: false,
        node_fetched: false,
        php_version: None,
        errors: Vec::new(),
    };
    let _ = paths.ensure_dirs();
    let statuses = detect(paths);
    let missing: Vec<Component> =
        statuses.iter().filter(|s| !s.present).map(|s| s.component).collect();

    let total = missing.len();
    let mut done = 0usize;

    // All stack components are downloaded now — nothing is installed via apt.
    report.apt_packages = Vec::new();

    // 1b. Install PHP from a static build (no apt/distro PHP) when missing.
    if missing.contains(&Component::Php) {
        sink.emit(ProgressEvent::Step { done, total, label: Component::Php.label().to_string() });
        match crate::php_static::install_php_static(paths, crate::php_versions::DEFAULT_PHP_VERSION, downloader, runner, sink) {
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
        done += 1;
    }

    // 1c. Install composer (downloaded, not apt) when missing. Must run after PHP so
    // the PHP binary is available for probing the composer version.
    if missing.contains(&Component::Composer) {
        sink.emit(ProgressEvent::Step { done, total, label: Component::Composer.label().to_string() });
        if let Err(e) = crate::php_cli::install_composer(paths, downloader, sink) {
            report.errors.push(format!("install composer: {e}"));
        } else {
            report.composer_fetched = true;
        }
        done += 1;
    }

    // 1d. Install mkcert from a static build (no apt) when missing.
    if missing.contains(&Component::Mkcert) {
        sink.emit(ProgressEvent::Step { done, total, label: Component::Mkcert.label().to_string() });
        match crate::mkcert_static::install_mkcert(paths, downloader, sink) {
            Ok(ver) => { report.mkcert_fetched = true; record_version(paths, "mkcert", &ver); }
            Err(e) => report.errors.push(format!("install mkcert: {e}")),
        }
        done += 1;
    }

    // 1e. Install nginx from a static build (no apt) when missing.
    if missing.contains(&Component::Nginx) {
        sink.emit(ProgressEvent::Step { done, total, label: Component::Nginx.label().to_string() });
        match crate::nginx_static::install_nginx(paths, downloader, sink) {
            Ok(ver) => { report.nginx_fetched = true; record_version(paths, "nginx", &ver); }
            Err(e) => report.errors.push(format!("install nginx: {e}")),
        }
        done += 1;
    }

    // 1f. Install redis (Valkey) from a static build (no apt) when missing.
    if missing.contains(&Component::Redis) {
        sink.emit(ProgressEvent::Step { done, total, label: Component::Redis.label().to_string() });
        match crate::redis_static::install_redis(paths, downloader, runner, sink) {
            Ok(ver) => { report.redis_fetched = true; record_version(paths, "redis", &ver); }
            Err(e) => report.errors.push(format!("install redis: {e}")),
        }
        done += 1;
    }

    // 1g. Install MariaDB from a static tarball (no apt) when missing.
    if missing.contains(&Component::Mariadb) {
        sink.emit(ProgressEvent::Step { done, total, label: Component::Mariadb.label().to_string() });
        match crate::mariadb_static::install_mariadb(paths, downloader, runner, sink) {
            Ok(ver) => { report.mariadb_fetched = true; record_version(paths, "mariadb", &ver); }
            Err(e) => report.errors.push(format!("install mariadb: {e}")),
        }
        done += 1;
    }

    // 1h. Install Node.js from the official static tarball (no apt) when missing.
    if missing.contains(&Component::Node) {
        sink.emit(ProgressEvent::Step { done, total, label: Component::Node.label().to_string() });
        match crate::node_static::install_node(paths, downloader, runner, sink) {
            Ok(ver) => { report.node_fetched = true; record_version(paths, "node", &ver); }
            Err(e) => report.errors.push(format!("install node: {e}")),
        }
        done += 1;
    }

    // apt auto-starts + enables the distro nginx/mariadb/redis systemd units, which
    // hold ports 80/3306/6379. Disable them so the app-managed processes can bind.
    let stack_units = stack_units_to_disable();
    if let Err(e) = privileged.disable_system_services(&stack_units) {
        report.errors.push(format!("disable system services: {e}"));
    }

    // 2. Fetch + extract mailpit (latest) into bin/mailpit/<version>/ when missing.
    if missing.contains(&Component::Mailpit) {
        sink.emit(ProgressEvent::Step { done, total, label: Component::Mailpit.label().to_string() });
        match crate::mailpit_static::install_mailpit(paths, downloader, runner, sink) {
            Ok(ver) => { report.mailpit_fetched = true; record_version(paths, "mailpit", &ver); }
            Err(e) => report.errors.push(format!("install mailpit: {e}")),
        }
        done += 1;
        let _ = done; // no further steps read `done`; suppress unused-assignment lint
    }

    // 3. Install the mkcert local CA (idempotent).
    match crate::bin::resolve_bin("mkcert", &crate::layout::managed_bin_dirs(paths)) {
        Some(mk) => match privileged.install_mkcert_ca(&mk) {
            Ok(()) => report.mkcert_ca = true,
            Err(e) => report.errors.push(format!("mkcert -install: {e}")),
        },
        None => report.errors.push("mkcert -install: mkcert not found".to_string()),
    }

    // 3b. Bundle certutil (NSS tools) and register the CA in the browser NSS
    // stores (Firefox/Chrome). No-apt: certutil is extracted from Ubuntu debs.
    match crate::certutil_static::install_certutil(paths, downloader, runner, sink) {
        Ok(certutil) => {
            report.certutil_fetched = true;
            if let Some(mk) = crate::bin::resolve_bin("mkcert", &crate::layout::managed_bin_dirs(paths)) {
                let bindir = certutil.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                let libdir = crate::certutil_static::certutil_lib_dir(paths);
                match crate::certutil_static::mkcert_install_nss(&mk, &bindir, &libdir) {
                    Ok(()) => report.mkcert_nss = true,
                    Err(e) => report.errors.push(format!("mkcert NSS install: {e}")),
                }
            }
        }
        Err(e) => report.errors.push(format!("install certutil: {e}")),
    }

    // 4. setcap the resolved nginx binary (same path the orchestrator spawns).
    if let Some(nginx) = resolve_bin("nginx", &crate::layout::managed_bin_dirs(paths)) {
        match privileged.setcap_nginx(&nginx) {
            Ok(()) => report.nginx_setcap = true,
            Err(e) => report.errors.push(format!("setcap nginx: {e}")),
        }
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
    use crate::paths::LaraluxPaths;
    use crate::privileged::FakePrivileged;

    #[test]
    fn parse_content_length_picks_last_case_insensitive() {
        let h = "HTTP/2 200\r\nContent-Length: 100\r\n\r\nHTTP/2 200\r\ncontent-length: 4096\r\n";
        assert_eq!(parse_content_length(h), 4096);
        assert_eq!(parse_content_length("no header here"), 0);
    }

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
    fn mkcert_has_no_apt_package() {
        assert!(apt_packages_for(Component::Mkcert).is_empty());
    }

    #[test]
    fn nginx_has_no_apt_package() {
        assert!(apt_packages_for(Component::Nginx).is_empty());
    }

    #[test]
    fn redis_has_no_apt_package() {
        assert!(apt_packages_for(Component::Redis).is_empty());
    }

    #[test]
    fn mariadb_has_no_apt_package() {
        assert!(apt_packages_for(Component::Mariadb).is_empty());
    }

    #[test]
    fn detect_reports_all_components() {
        let paths = LaraluxPaths::new(std::env::temp_dir().join(format!("lara-detect-{}", std::process::id())));
        let statuses = detect(&paths);
        assert_eq!(statuses.len(), 8);
        assert!(!statuses.iter().find(|s| s.component == Component::Mailpit).unwrap().present);
    }

    #[test]
    fn run_setup_disables_distro_stack_services() {
        let root = std::env::temp_dir().join(format!("lara-disable-{}", std::process::id()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let paths = LaraluxPaths::new(root.clone());
        let priv_ = FakePrivileged::new();
        let disabled = priv_.disabled_services();
        let dl = FakeDownloader::new();
        let runner = crate::scaffold::FakeCommandRunner::new();

        let _ = run_setup(&paths, &priv_, &dl, &runner, &crate::progress::NullProgress);

        let calls = disabled.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let units = &calls[0];
        assert!(units.contains(&"nginx".to_string()));
        assert!(units.contains(&"mariadb".to_string()));
        assert!(units.contains(&"redis-server".to_string()));
        std::fs::remove_dir_all(&root).ok();
    }



    #[test]
    fn composer_is_a_component_with_no_apt_package() {
        assert!(Component::ALL.contains(&Component::Composer));
        assert!(apt_packages_for(Component::Composer).is_empty());
        assert_eq!(Component::Composer.label(), "composer");
    }

    struct FakeProgress(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl crate::progress::ProgressSink for FakeProgress {
        fn emit(&self, ev: crate::progress::ProgressEvent) {
            if let crate::progress::ProgressEvent::Step { done, total, label } = ev {
                self.0.lock().unwrap().push(format!("{done}/{total}:{label}"));
            }
        }
    }

    #[test]
    fn run_setup_emits_a_step_per_missing_component() {
        let root = std::env::temp_dir().join(format!("lara-setup-prog-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        paths.ensure_dirs().unwrap();
        let priv_ = FakePrivileged::new();
        let dl = FakeDownloader::new();
        let runner = crate::scaffold::FakeCommandRunner::new();
        let steps = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = FakeProgress(steps.clone());
        let _ = run_setup(&paths, &priv_, &dl, &runner, &sink);
        // At least the PHP/mailpit/composer steps fire (all components are missing on a fresh root).
        assert!(!steps.lock().unwrap().is_empty(), "expected Step events");
        assert!(steps.lock().unwrap().iter().all(|s| s.contains('/')));
        std::fs::remove_dir_all(&root).ok();
    }
}
