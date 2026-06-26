use crate::paths::LaraluxPaths;
use crate::service::ServiceKind;
use std::path::PathBuf;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedTool { Php, Nginx, Mariadb, Redis, Mailpit, Mkcert, Composer }

impl ManagedTool {
    pub const ALL: [ManagedTool; 7] = [
        ManagedTool::Php, ManagedTool::Nginx, ManagedTool::Mariadb, ManagedTool::Redis,
        ManagedTool::Mailpit, ManagedTool::Mkcert, ManagedTool::Composer,
    ];
}

pub struct ToolInfo {
    pub key: &'static str,
    pub display: &'static str,
    pub cli_binary: Option<&'static str>,
    pub service_kind: Option<ServiceKind>,
}

pub fn info(tool: ManagedTool) -> ToolInfo {
    use ManagedTool::*;
    match tool {
        Php => ToolInfo { key: "php", display: "PHP", cli_binary: Some("php"), service_kind: Some(ServiceKind::PhpFpm) },
        Nginx => ToolInfo { key: "nginx", display: "Nginx", cli_binary: Some("nginx"), service_kind: Some(ServiceKind::Nginx) },
        Mariadb => ToolInfo { key: "mariadb", display: "MariaDB", cli_binary: Some("mariadb"), service_kind: Some(ServiceKind::Mariadb) },
        Redis => ToolInfo { key: "redis", display: "Redis", cli_binary: Some("redis-cli"), service_kind: Some(ServiceKind::Redis) },
        Mailpit => ToolInfo { key: "mailpit", display: "Mailpit", cli_binary: None, service_kind: Some(ServiceKind::Mailpit) },
        Mkcert => ToolInfo { key: "mkcert", display: "mkcert", cli_binary: Some("mkcert"), service_kind: None },
        Composer => ToolInfo { key: "composer", display: "Composer", cli_binary: Some("composer"), service_kind: None },
    }
}

pub fn key(tool: ManagedTool) -> &'static str {
    info(tool).key
}

pub fn from_key(k: &str) -> Option<ManagedTool> {
    ManagedTool::ALL.into_iter().find(|t| key(*t) == k)
}

/// Absolute path to the tool's terminal CLI under `bin/<key>/current/<cli>`, if it has one.
pub fn cli_path(tool: ManagedTool, paths: &LaraluxPaths) -> Option<PathBuf> {
    info(tool)
        .cli_binary
        .map(|b| paths.bin().join(key(tool)).join("current").join(b))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolVersion {
    pub version: String,
    pub installed: bool,
    pub active: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("install failed: {0}")]
    Install(String),
}

/// A multi-version tool's catalog: the curated `known` list unioned with what's
/// installed, newest-first, with `installed`/`active` flags set.
fn known_catalog(known: &[&str], installed: Vec<String>, active: &str) -> Vec<ToolVersion> {
    let mut versions: Vec<String> = known.iter().map(|s| s.to_string()).collect();
    for v in &installed {
        if !versions.contains(v) {
            versions.push(v.clone());
        }
    }
    let vkey = |v: &str| v.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    versions.sort_by(|a, b| vkey(b).cmp(&vkey(a)));
    versions
        .into_iter()
        .map(|v| ToolVersion { installed: installed.contains(&v), active: v == active, version: v })
        .collect()
}

pub fn available_versions(tool: ManagedTool, paths: &LaraluxPaths) -> Vec<ToolVersion> {
    let cfg = crate::config::Config::load(&paths.config_file()).unwrap_or_default();
    match tool {
        ManagedTool::Php => crate::php_versions::php_versions(paths, &cfg.php_version)
            .into_iter()
            .map(|p| ToolVersion { version: p.version, installed: p.installed, active: p.active })
            .collect(),
        ManagedTool::Nginx => known_catalog(
            &crate::nginx_static::KNOWN_NGINX_VERSIONS,
            crate::layout::installed_versions(paths, "nginx"),
            &cfg.versions.get("nginx").cloned().unwrap_or_default(),
        ),
        ManagedTool::Mariadb => known_catalog(
            &crate::mariadb_static::KNOWN_MARIADB_VERSIONS,
            crate::layout::installed_versions(paths, "mariadb"),
            &cfg.versions.get("mariadb").cloned().unwrap_or_default(),
        ),
        ManagedTool::Redis => known_catalog(
            &crate::redis_static::KNOWN_REDIS_VERSIONS,
            crate::layout::installed_versions(paths, "redis"),
            &cfg.versions.get("redis").cloned().unwrap_or_default(),
        ),
        ManagedTool::Mailpit => known_catalog(
            &crate::mailpit_static::KNOWN_MAILPIT_VERSIONS,
            crate::layout::installed_versions(paths, "mailpit"),
            &cfg.versions.get("mailpit").cloned().unwrap_or_default(),
        ),
        ManagedTool::Mkcert => known_catalog(
            &crate::mkcert_static::KNOWN_MKCERT_VERSIONS,
            crate::layout::installed_versions(paths, "mkcert"),
            &cfg.versions.get("mkcert").cloned().unwrap_or_default(),
        ),
        ManagedTool::Composer => known_catalog(
            &crate::php_cli::KNOWN_COMPOSER_VERSIONS,
            crate::layout::installed_versions(paths, "composer"),
            &cfg.versions.get("composer").cloned().unwrap_or_default(),
        ),
    }
}

/// Install a specific version of a tool by dispatching to its installer.
pub fn install_version(
    tool: ManagedTool,
    paths: &LaraluxPaths,
    version: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
    sink: &dyn ProgressSink,
) -> Result<String, ToolError> {
    match tool {
        ManagedTool::Php => crate::php_static::install_php_static(paths, version, downloader, runner, sink)
            .map_err(|e| ToolError::Install(e.to_string())),
        ManagedTool::Nginx => crate::nginx_static::install_nginx_version(paths, version, downloader, sink)
            .map_err(|e| ToolError::Install(e.to_string())),
        ManagedTool::Mariadb => crate::mariadb_static::install_mariadb_version(paths, version, downloader, runner, sink)
            .map_err(|e| ToolError::Install(e.to_string())),
        ManagedTool::Redis => crate::redis_static::install_redis_version(paths, version, downloader, runner, sink)
            .map_err(|e| ToolError::Install(e.to_string())),
        ManagedTool::Mailpit => crate::mailpit_static::install_mailpit_version(paths, version, downloader, runner, sink)
            .map_err(|e| ToolError::Install(e.to_string())),
        ManagedTool::Mkcert => crate::mkcert_static::install_mkcert_version(paths, version, downloader, sink)
            .map_err(|e| ToolError::Install(e.to_string())),
        ManagedTool::Composer => crate::php_cli::install_composer_version(paths, version, downloader, sink)
            .map_err(|e| ToolError::Install(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_binary_mapping_and_mailpit_has_none() {
        assert_eq!(info(ManagedTool::Php).cli_binary, Some("php"));
        assert_eq!(info(ManagedTool::Composer).cli_binary, Some("composer"));
        assert_eq!(info(ManagedTool::Mariadb).cli_binary, Some("mariadb"));
        assert_eq!(info(ManagedTool::Mkcert).cli_binary, Some("mkcert"));
        assert_eq!(info(ManagedTool::Redis).cli_binary, Some("redis-cli"));
        assert_eq!(info(ManagedTool::Nginx).cli_binary, Some("nginx"));
        assert_eq!(info(ManagedTool::Mailpit).cli_binary, None);
    }

    #[test]
    fn key_roundtrips_through_from_key() {
        for t in ManagedTool::ALL {
            assert_eq!(from_key(key(t)), Some(t));
        }
        assert_eq!(from_key("nope"), None);
    }

    #[test]
    fn cli_path_is_under_current_and_none_for_mailpit() {
        let p = LaraluxPaths::new("/tmp/lara".into());
        assert_eq!(cli_path(ManagedTool::Php, &p), Some(PathBuf::from("/tmp/lara/bin/php/current/php")));
        assert_eq!(cli_path(ManagedTool::Redis, &p), Some(PathBuf::from("/tmp/lara/bin/redis/current/redis-cli")));
        assert_eq!(cli_path(ManagedTool::Mailpit, &p), None);
    }

    #[test]
    fn php_available_versions_lists_known_set() {
        let root = std::env::temp_dir().join(format!("lara-tools-php-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        paths.ensure_dirs().unwrap();
        let vs = available_versions(ManagedTool::Php, &paths);
        // KNOWN_PHP_VERSIONS has 6 entries (8.0..8.5); none installed on a fresh root.
        assert_eq!(vs.len(), 6);
        assert!(vs.iter().all(|v| !v.installed));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn composer_available_versions_includes_known_set_newest_first() {
        let root = std::env::temp_dir().join(format!("lara-tools-cmp-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        std::fs::create_dir_all(paths.version_dir("composer", "2.6.6")).unwrap();
        let vs = available_versions(ManagedTool::Composer, &paths);
        assert_eq!(vs.len(), crate::php_cli::KNOWN_COMPOSER_VERSIONS.len());
        assert!(vs.iter().find(|v| v.version == "2.6.6").unwrap().installed);
        assert_eq!(vs[0].version, "2.8.9"); // newest first
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn nginx_available_versions_includes_known_set_and_marks_installed() {
        let root = std::env::temp_dir().join(format!("lara-tools-ng-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        // Seed one installed nginx version; the catalog should still list the full known set.
        std::fs::create_dir_all(paths.version_dir("nginx", "1.30.3")).unwrap();
        let vs = available_versions(ManagedTool::Nginx, &paths);
        assert_eq!(vs.len(), crate::nginx_static::KNOWN_NGINX_VERSIONS.len());
        assert!(vs.iter().find(|v| v.version == "1.30.3").unwrap().installed);
        assert!(!vs.iter().find(|v| v.version == "1.26.3").unwrap().installed);
        // Newest first.
        assert_eq!(vs[0].version, "1.31.2");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn mariadb_available_versions_includes_known_set_newest_first() {
        let root = std::env::temp_dir().join(format!("lara-tools-mdb-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        std::fs::create_dir_all(paths.version_dir("mariadb", "10.11.10")).unwrap();
        let vs = available_versions(ManagedTool::Mariadb, &paths);
        assert_eq!(vs.len(), crate::mariadb_static::KNOWN_MARIADB_VERSIONS.len());
        assert!(vs.iter().find(|v| v.version == "10.11.10").unwrap().installed);
        assert_eq!(vs[0].version, "11.8.2"); // newest first
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn redis_available_versions_includes_known_set_newest_first() {
        let root = std::env::temp_dir().join(format!("lara-tools-rds-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        std::fs::create_dir_all(paths.version_dir("redis", "8.0.4")).unwrap();
        let vs = available_versions(ManagedTool::Redis, &paths);
        assert_eq!(vs.len(), crate::redis_static::KNOWN_REDIS_VERSIONS.len());
        assert!(vs.iter().find(|v| v.version == "8.0.4").unwrap().installed);
        assert_eq!(vs[0].version, "9.1.0"); // newest first
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn mailpit_available_versions_includes_known_set_newest_first() {
        let root = std::env::temp_dir().join(format!("lara-tools-mpv-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        std::fs::create_dir_all(paths.version_dir("mailpit", "1.25.0")).unwrap();
        let vs = available_versions(ManagedTool::Mailpit, &paths);
        assert_eq!(vs.len(), crate::mailpit_static::KNOWN_MAILPIT_VERSIONS.len());
        assert!(vs.iter().find(|v| v.version == "1.25.0").unwrap().installed);
        assert_eq!(vs[0].version, "1.30.2"); // newest first
        std::fs::remove_dir_all(&root).ok();
    }
}
